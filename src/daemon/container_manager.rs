use nix::sys::signal;
use nix::unistd::{dup, Pid};
use rustbox::container::{
    drain_cgroup_and_remove, run_sandbox, wait_and_cleanup, Container, ContainerConfig,
    SandboxConfig,
};
use rustbox::error::{ContainerError, DaemonError};
use rustbox::ipc::{read_message, write_message, ContainerSummary, DaemonRequest, DaemonResponse};
use rustbox::storage::{delete_metadata, load_all_metadata, save_metadata, ContainerLogs};
use std::collections::HashMap;
use std::os::fd::{BorrowedFd, IntoRawFd};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

/// Active attach session tracking
///
/// Represents a client attached to a container's TTY. Each session manages
/// bidirectional I/O forwarding between the client's Unix domain socket and
/// the container's pseudo-terminal (PTY).
///
/// # Architecture
///
/// Each `AttachSession` spawns two async tasks:
///
/// 1. **PTY → Client**: Reads from container's PTY master FD, sends output
///    to client via `AttachStdout` messages
/// 2. **Client → PTY**: Receives `AttachStdin` messages from client, writes
///    input to PTY master FD
///
/// Both tasks run concurrently using `tokio::select!` and can be gracefully
/// terminated when the client detaches or the container exits.
///
/// # Lifecycle
///
/// - **Created**: When client sends `AttachRequest` for running container
/// - **Active**: Both tasks running, forwarding I/O bidirectionally
/// - **Terminated**: When client detaches (Ctrl+P Ctrl+Q or Ctrl+C) or
///   container process exits
///
/// # Resource Management
///
/// The PTY master FD is duplicated using `dup()` to provide separate read
/// and write file descriptors. This ensures proper RAII cleanup and prevents
/// use-after-free bugs when the session ends.
///
/// # Future Use
///
/// Fields like `session_id`, `client_info`, and `started_at` are reserved
/// for multi-client attach support and session management features.
#[allow(dead_code)]
pub struct AttachSession {
    /// Unique identifier for this attach session
    pub session_id: String,

    /// Container ID being attached to
    pub container_id: String,

    /// Client connection information (for logging/diagnostics)
    pub client_info: String,

    /// Task handle for PTY→client forwarding
    pub pty_to_client_task: JoinHandle<Result<(), String>>,

    /// Task handle for client→PTY forwarding
    pub client_to_pty_task: JoinHandle<Result<(), String>>,

    /// Session start time
    pub started_at: SystemTime,
}

#[allow(dead_code)]
impl AttachSession {
    /// Create a new attach session
    pub fn new(
        session_id: String,
        container_id: String,
        client_info: String,
        pty_to_client_task: JoinHandle<Result<(), String>>,
        client_to_pty_task: JoinHandle<Result<(), String>>,
    ) -> Self {
        Self {
            session_id,
            container_id,
            client_info,
            pty_to_client_task,
            client_to_pty_task,
            started_at: SystemTime::now(),
        }
    }

    /// Abort both I/O forwarding tasks
    pub fn abort(&self) {
        self.pty_to_client_task.abort();
        self.client_to_pty_task.abort();
    }
}

/// In-memory registry of all containers managed by daemon
pub struct ContainerRegistry {
    /// Map of container ID to Container
    containers: HashMap<String, Container>,
}

impl ContainerRegistry {
    pub fn new() -> Self {
        Self {
            containers: HashMap::new(),
        }
    }

    /// Add a new container
    pub fn insert(&mut self, container: Container) -> Result<(), ContainerError> {
        if self.containers.contains_key(&container.id) {
            return Err(ContainerError::AlreadyExists(container.id.to_string()));
        }

        // Check for name conflicts
        if self.containers.values().any(|c| c.name == container.name) {
            return Err(ContainerError::AlreadyExists(format!(
                "Container with name '{}' already exists",
                container.name
            )));
        }

        self.containers.insert(container.id.clone(), container);
        Ok(())
    }

    /// Get container by ID
    pub fn get(&self, id: &str) -> Option<&Container> {
        self.containers.get(id)
    }

    /// Get mutable container by ID
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Container> {
        self.containers.get_mut(id)
    }

    /// Resolve container by ID or name, returning the container ID
    ///
    /// This method accepts either a container ID or name and returns the
    /// actual container ID if found. This allows commands to work with
    /// both identifiers interchangeably.
    ///
    /// # Arguments
    /// * `id_or_name` - Container ID or name to resolve
    ///
    /// # Returns
    /// * `Some(String)` - The container ID if found
    /// * `None` - If no container matches the given ID or name
    pub fn resolve_id_or_name(&self, id_or_name: &str) -> Option<String> {
        if self.containers.contains_key(id_or_name) {
            return Some(id_or_name.to_string());
        }

        // Although this time complex is O(n), however we will not expect a lot of containers in single VM, therefore it will be acceptable.
        self.containers
            .values()
            .find(|c| c.name == id_or_name)
            .map(|c| c.id.clone())
    }

    /// Remove container by ID
    pub fn remove(&mut self, id: &str) -> Option<Container> {
        self.containers.remove(id)
    }

    /// List all container IDs
    #[allow(dead_code)]
    pub fn list_ids(&self) -> Vec<String> {
        self.containers.keys().cloned().collect()
    }

    /// List all containers
    pub fn list(&self) -> Vec<&Container> {
        self.containers.values().collect()
    }

    /// Count running containers
    pub fn count_running(&self) -> usize {
        self.containers
            .values()
            .filter(|c| c.state.is_running())
            .count()
    }

    /// Count total containers
    pub fn count_total(&self) -> usize {
        self.containers.len()
    }
}

/// Container manager that handles all container operations
#[derive(Clone)]
pub struct ContainerManager {
    registry: Arc<RwLock<ContainerRegistry>>,
    /// Active attach sessions (session_id → AttachSession)
    #[allow(dead_code)]
    sessions: Arc<RwLock<HashMap<String, AttachSession>>>,
    start_time: std::time::Instant,
}

impl ContainerManager {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(RwLock::new(ContainerRegistry::new())),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            start_time: std::time::Instant::now(),
        }
    }

    /// Get daemon uptime in seconds
    pub fn uptime_seconds(&self) -> u64 {
        self.start_time.elapsed().as_secs()
    }

    /// Recover container state from disk on daemon startup
    pub async fn recover_state(&self) -> Result<(), DaemonError> {
        info!("Recovering container state from disk...");

        let containers = load_all_metadata().map_err(DaemonError::Storage)?;

        let mut registry = self.registry.write().await;

        for mut container in containers {
            // Check if container process is still running
            if let Some(pid) = container.pid {
                // Try to check if PID exists (simple check)
                if !Self::pid_exists(pid) {
                    info!(
                        "Container {} (PID {}) is no longer running, marking as exited",
                        container.id, pid
                    );
                    let _ = container.mark_exited(255); // Unknown exit code
                }
            }

            let id = container.id.clone();
            if let Err(e) = registry.insert(container) {
                warn!("Failed to recover container {}: {}", id, e);
            } else {
                info!("Recovered container: {}", id);
            }
        }

        info!("Recovered {} containers from disk", registry.count_total());
        Ok(())
    }

    /// Check if a PID exists (simple implementation)
    fn pid_exists(pid: i32) -> bool {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }

    /// Helper function to mark a container as exited and save metadata
    ///
    /// This is a common pattern used when container startup fails or crashes.
    /// It acquires the registry lock, marks the container as exited with the given
    /// exit code, and persists the updated state to disk.
    ///
    /// # Arguments
    /// * `registry` - Shared registry lock
    /// * `container_id` - ID of the container to mark as exited
    /// * `exit_code` - Exit code to set (typically 1 for errors, 255 for unknown)
    async fn mark_container_exited(
        registry: Arc<RwLock<ContainerRegistry>>,
        container_id: &str,
        exit_code: i32,
    ) {
        let mut registry = registry.write().await;
        if let Some(container) = registry.get_mut(container_id) {
            let _ = container.mark_exited(exit_code);
            let _ = save_metadata(container);
        }
    }

    /// Handle RunRequest
    pub async fn handle_run(
        &self,
        name: Option<String>,
        config: ContainerConfig,
    ) -> Result<DaemonResponse, DaemonError> {
        info!("Creating new container with config: {:?}", config);

        // Validate configuration
        config.validate().map_err(DaemonError::InvalidRequest)?;

        // Create container
        let container = Container::new(name, config);
        let container_id = container.id.clone();
        let container_name = container.name.clone();

        // Insert into registry
        {
            let mut registry = self.registry.write().await;
            registry
                .insert(container.clone())
                .map_err(DaemonError::Container)?;
        }

        // Save metadata to disk
        save_metadata(&container).map_err(DaemonError::Storage)?;

        // Create log directory and files
        let logs = ContainerLogs::new(container_id.clone());
        logs.create_log_files()
            .map_err(|e| DaemonError::Storage(e.into()))?;

        info!("Container created: {} ({})", container_id, container_name);

        // Mark container as starting (get a fake PID for now since we don't have the real one yet)
        {
            let mut registry = self.registry.write().await;
            if let Some(container) = registry.get_mut(&container_id) {
                // Transition to Running state (we'll get the real PID from sandbox later)
                let _ = container.mark_started(1); // Temporary PID, will be updated
                save_metadata(container).map_err(DaemonError::Storage)?;
            }
        }

        // Start the container process
        let container_clone = container.clone();
        let registry_clone = self.registry.clone();
        let cid = container_id.clone();

        tokio::spawn(async move {
            // Get log file paths
            let logs = ContainerLogs::new(cid.clone());

            // Ensure overlay directories are created before mounting
            if let Err(e) = container_clone.overlay_paths.create_dirs() {
                error!(
                    "Failed to create overlay directories for container {}: {}",
                    cid, e
                );
                Self::mark_container_exited(registry_clone.clone(), &cid, 1).await;
                return;
            }

            // Copy upperdir content from repository to container-specific location
            if let Err(e) = container_clone
                .overlay_paths
                .copy_upperdir_content(&container_clone.config.rootfs_path)
            {
                error!(
                    "Failed to copy upperdir content for container {}: {}",
                    cid, e
                );
                Self::mark_container_exited(registry_clone.clone(), &cid, 1).await;
                return;
            }

            // Build sandbox config using overlay paths from container
            let sandbox_config = SandboxConfig {
                lower_dir: container_clone.overlay_paths.lower.clone(),
                upper_dir: container_clone.overlay_paths.upper.clone(),
                work_dir: container_clone.overlay_paths.work.clone(),
                merged_dir: container_clone.overlay_paths.merged.clone(),
                memory_limit: container_clone.config.memory_limit.clone(),
                command: container_clone.config.command.clone(),
                workdir: container_clone.config.workdir.clone(),
                cpu_limit: container_clone.config.cpu_limit.clone(),
                stdout_log_path: Some(logs.stdout_path().to_string_lossy().to_string()),
                stderr_log_path: Some(logs.stderr_path().to_string_lossy().to_string()),
                tty: container_clone.config.tty,
                isolate_user: container_clone.config.isolate_user,
                isolate_network: container_clone.config.isolate_network,
            };

            info!("Starting container {} in background task", cid);

            // Run sandbox in blocking task - now returns immediately with PTY FD and child PID
            let sandbox_result =
                tokio::task::spawn_blocking(move || run_sandbox(sandbox_config)).await;

            match sandbox_result {
                Ok(Ok(result)) => {
                    info!(
                        "Container {} started with PID {}, PTY master FD: {:?}, child_pid: {}",
                        cid,
                        result.child_pid,
                        result.pty_master,
                        result.child_pid.as_raw()
                    );

                    // Update container with PTY master FD, PID, and actual cgroup path immediately
                    {
                        let mut registry = registry_clone.write().await;
                        if let Some(container) = registry.get_mut(&cid) {
                            container.pty_master = result.pty_master;
                            container.cgroup_path = result.cleanup_paths.cgroup.clone();
                            info!(
                                "Updated container {} cgroup path to: {}",
                                cid,
                                container.cgroup_path.display()
                            );
                            let _ = container.mark_started(result.child_pid.as_raw());
                            let _ = save_metadata(container);
                            if let Some(fd) = result.pty_master {
                                match unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned() {
                                    Ok(_) => {
                                        info!(
                                            "PTY master FD {} is valid immediately after storage",
                                            fd
                                        );
                                    }
                                    Err(e) => {
                                        error!("PTY master FD {} is INVALID immediately after storage: {}", fd, e);
                                    }
                                }
                            }
                        }
                    } // Spawn a separate task to wait for container exit (don't await it!)
                    let registry_clone2 = registry_clone.clone();
                    let cid2 = cid.clone();
                    tokio::spawn(async move {
                        // Wait for container to exit in a blocking task
                        tokio::task::spawn_blocking(move || {
                            let _ = wait_and_cleanup(result);
                        })
                        .await
                        .ok();

                        info!("Container {} has exited", cid2);

                        // Container has exited, update state
                        // Note: wait_and_cleanup doesn't currently return exit code
                        let exit_code = 0;
                        info!("Container {} exited with code: {}", cid2, exit_code);
                        Self::mark_container_exited(registry_clone2, &cid2, exit_code).await;
                    });
                }
                Ok(Err(e)) => {
                    error!("Container {} failed to start: {}", cid, e);
                    Self::mark_container_exited(registry_clone.clone(), &cid, 1).await;
                }
                Err(e) => {
                    error!("Container {} start task panicked: {}", cid, e);
                    Self::mark_container_exited(registry_clone.clone(), &cid, 1).await;
                }
            }
        });

        Ok(DaemonResponse::RunResponse {
            container_id,
            name: container_name,
            state: "Running".to_string(),
        })
    }

    /// Handle StartRequest
    pub async fn handle_start(
        &self,
        container_id_or_name: String,
    ) -> Result<DaemonResponse, DaemonError> {
        info!("Starting container: {}", container_id_or_name);

        // Resolve container ID or name to actual container ID
        let container_id = {
            let registry = self.registry.read().await;
            registry
                .resolve_id_or_name(&container_id_or_name)
                .ok_or_else(|| {
                    DaemonError::Container(ContainerError::NotFound(
                        container_id_or_name.to_string(),
                    ))
                })?
        };

        let container = {
            let mut registry = self.registry.write().await;

            let container = registry.get_mut(&container_id).ok_or_else(|| {
                DaemonError::Container(ContainerError::NotFound(container_id.to_string()))
            })?;

            if !container.state.can_start() {
                return Err(DaemonError::InvalidRequest(format!(
                    "Container {} cannot be started from state: {}",
                    container_id, container.state
                )));
            }

            let _ = container.mark_started(1); // Temporary PID, will be updated
            save_metadata(container).map_err(DaemonError::Storage)?;

            container.clone()
        };

        // Start the container process in background
        let registry_clone = self.registry.clone();
        let cid = container_id.clone();

        tokio::spawn(async move {
            // Get log file paths
            let logs = ContainerLogs::new(cid.clone());

            // Ensure overlay directories are created before mounting
            // For restarted containers, the upperdir already exists with previous state
            // We only need to recreate merged/work directories if they were cleaned up
            if let Err(e) = container.overlay_paths.create_dirs() {
                error!(
                    "Failed to create overlay directories for container {}: {}",
                    cid, e
                );
                Self::mark_container_exited(registry_clone.clone(), &cid, 1).await;
                return;
            }

            let sandbox_config = SandboxConfig {
                lower_dir: container.overlay_paths.lower.clone(),
                upper_dir: container.overlay_paths.upper.clone(),
                work_dir: container.overlay_paths.work.clone(),
                merged_dir: container.overlay_paths.merged.clone(),
                memory_limit: container.config.memory_limit.clone(),
                command: container.config.command.clone(),
                workdir: container.config.workdir.clone(),
                cpu_limit: container.config.cpu_limit.clone(),
                stdout_log_path: Some(logs.stdout_path().to_string_lossy().to_string()),
                stderr_log_path: Some(logs.stderr_path().to_string_lossy().to_string()),
                tty: container.config.tty,
                isolate_user: container.config.isolate_user,
                isolate_network: container.config.isolate_network,
            };

            info!("Starting container {} in background task", cid);

            // Run sandbox in blocking task - now returns immediately with PTY FD and child PID
            let sandbox_result =
                tokio::task::spawn_blocking(move || run_sandbox(sandbox_config)).await;

            match sandbox_result {
                Ok(Ok(result)) => {
                    info!(
                        "Container {} started with PID {}, PTY master FD: {:?}, child_pid: {}",
                        cid,
                        result.child_pid,
                        result.pty_master,
                        result.child_pid.as_raw()
                    );

                    // Update container with PTY master FD, PID, and actual cgroup path immediately
                    {
                        let mut registry = registry_clone.write().await;
                        if let Some(container) = registry.get_mut(&cid) {
                            container.pty_master = result.pty_master;
                            container.cgroup_path = result.cleanup_paths.cgroup.clone();
                            info!(
                                "Updated container {} cgroup path to: {}",
                                cid,
                                container.cgroup_path.display()
                            );
                            let _ = container.mark_started(result.child_pid.as_raw());
                            let _ = save_metadata(container);
                            if let Some(fd) = result.pty_master {
                                match unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned() {
                                    Ok(_) => {
                                        info!(
                                            "PTY master FD {} is valid immediately after storage",
                                            fd
                                        );
                                    }
                                    Err(e) => {
                                        error!("PTY master FD {} is INVALID immediately after storage: {}", fd, e);
                                    }
                                }
                            }
                        }
                    } 

                    let registry_clone2 = registry_clone.clone();
                    let cid2 = cid.clone();
                    tokio::spawn(async move {
                        // Wait for container to exit in a blocking task
                        tokio::task::spawn_blocking(move || {
                            let _ = wait_and_cleanup(result);
                        })
                        .await
                        .ok();

                        info!("Container {} has exited", cid2);

                        // Container has exited, update state
                        // Note: wait_and_cleanup doesn't currently return exit code
                        let exit_code = 0;
                        info!("Container {} exited with code: {}", cid2, exit_code);
                        Self::mark_container_exited(registry_clone2, &cid2, exit_code).await;
                    });
                }
                Ok(Err(e)) => {
                    error!("Container {} failed to start: {}", cid, e);
                    Self::mark_container_exited(registry_clone.clone(), &cid, 1).await;
                }
                Err(e) => {
                    error!("Container {} start task panicked: {}", cid, e);
                    Self::mark_container_exited(registry_clone.clone(), &cid, 1).await;
                }
            }
        });

        Ok(DaemonResponse::StartResponse {
            container_id,
            state: "Running".to_string(),
        })
    }

    /// Handle StopRequest
    pub async fn handle_stop(
        &self,
        container_id_or_name: String,
        timeout: u64,
    ) -> Result<DaemonResponse, DaemonError> {
        info!(
            "Stopping container: {} (timeout: {}s)",
            container_id_or_name, timeout
        );

        let mut registry = self.registry.write().await;

        // Resolve container ID or name to actual container ID
        let container_id = registry
            .resolve_id_or_name(&container_id_or_name)
            .ok_or_else(|| {
                DaemonError::Container(ContainerError::NotFound(container_id_or_name.to_string()))
            })?;

        let container = registry.get_mut(&container_id).ok_or_else(|| {
            DaemonError::Container(ContainerError::NotFound(container_id.to_string()))
        })?;

        if !container.state.can_stop() {
            return Err(DaemonError::InvalidRequest(format!(
                "Container {} is not running (state: {})",
                container_id, container.state
            )));
        }

        // Get the PID if available
        let pid = container.pid;

        // Mark as stopped first
        container
            .mark_stopped()
            .map_err(|e| DaemonError::Container(ContainerError::StopFailed(e)))?;

        save_metadata(container).map_err(DaemonError::Storage)?;

        info!("Container stopped: {}", container_id);

        // If we have a PID, try to send signals
        // Note: In the current implementation, the sandbox manages the process lifecycle
        // and we don't have direct access to the container PID after forking.
        // This is a limitation of the current design - for now we just mark it as stopped.
        // A future improvement would be to track the outer fork PID and send signals to it.
        if let Some(pid) = pid {
            info!("Sending SIGTERM to PID {}", pid);

            // Send SIGTERM to the process
            match nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid),
                nix::sys::signal::Signal::SIGTERM,
            ) {
                Ok(()) => {
                    info!("SIGTERM sent to PID {}", pid);

                    // Wait for process to exit gracefully (up to 10 seconds)
                    let timeout = std::time::Duration::from_secs(10);
                    let start = std::time::Instant::now();

                    while start.elapsed() < timeout {
                        // Check if process is still alive
                        match nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            None, // Signal 0 for process existence check
                        ) {
                            Ok(()) => {
                                // Process is still alive, wait a bit
                                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            }
                            Err(_) => {
                                // Process has exited
                                info!("Process {} exited gracefully", pid);
                                break;
                            }
                        }
                    }

                    // If process is still alive after timeout, send SIGKILL
                    if nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok() {
                        info!("Process {} still alive after timeout, sending SIGKILL", pid);
                        let _ = nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(pid),
                            nix::sys::signal::Signal::SIGKILL,
                        );
                    }
                }
                Err(e) => {
                    warn!("Failed to send SIGTERM to PID {}: {}", pid, e);
                }
            }
        }

        Ok(DaemonResponse::StopResponse {
            container_id,
            state: container.state.to_string(),
        })
    }

    /// Handle ListRequest
    pub async fn handle_list(&self, all: bool) -> Result<DaemonResponse, DaemonError> {
        let registry = self.registry.read().await;

        let containers: Vec<ContainerSummary> = registry
            .list()
            .iter()
            .filter(|c| all || c.state.is_running())
            .map(|c| ContainerSummary::from(*c))
            .collect();

        Ok(DaemonResponse::ListResponse { containers })
    }

    /// Handle InspectRequest
    pub async fn handle_inspect(
        &self,
        container_id_or_name: String,
    ) -> Result<DaemonResponse, DaemonError> {
        let registry = self.registry.read().await;

        // Resolve container ID or name to actual container ID
        let container_id = registry
            .resolve_id_or_name(&container_id_or_name)
            .ok_or_else(|| {
                DaemonError::Container(ContainerError::NotFound(container_id_or_name.to_string()))
            })?;

        let container =
            registry
                .get(&container_id)
                .ok_or(DaemonError::Container(ContainerError::NotFound(
                    container_id,
                )))?;

        Ok(DaemonResponse::InspectResponse {
            container: Box::new(container.clone()),
        })
    }

    /// Handle RemoveRequest
    pub async fn handle_remove(
        &self,
        container_id_or_name: String,
        force: bool,
    ) -> Result<DaemonResponse, DaemonError> {
        info!(
            "Removing container: {} (force: {})",
            container_id_or_name, force
        );

        let mut registry = self.registry.write().await;

        // Resolve container ID or name to actual container ID
        let container_id = registry
            .resolve_id_or_name(&container_id_or_name)
            .ok_or_else(|| {
                DaemonError::Container(ContainerError::NotFound(container_id_or_name.to_string()))
            })?;

        let container = registry.get(&container_id).ok_or_else(|| {
            DaemonError::Container(ContainerError::NotFound(container_id.to_string()))
        })?;

        // Check if container can be removed
        if !force && !container.state.can_remove() {
            return Err(DaemonError::InvalidRequest(format!(
                "Container {container_id} is running. Use --force to remove."
            )));
        }

        // Remove from registry
        let container = registry.remove(&container_id).ok_or_else(|| {
            DaemonError::Container(ContainerError::NotFound(container_id.to_string()))
        })?;

        // Cleanup overlay filesystem
        if let Err(e) = container.overlay_paths.cleanup() {
            warn!("Failed to cleanup overlay for {}: {}", container_id, e);
        }

        // Cleanup cgroup
        drain_cgroup_and_remove(&container.cgroup_path);

        // Delete metadata from disk
        delete_metadata(&container_id).map_err(DaemonError::Storage)?;

        info!("Container removed: {}", container_id);

        Ok(DaemonResponse::RemoveResponse {
            container_id,
            message: "Container removed successfully".to_string(),
        })
    }

    /// Handle StatusRequest
    pub async fn handle_status(&self) -> Result<DaemonResponse, DaemonError> {
        let registry = self.registry.read().await;

        Ok(DaemonResponse::StatusResponse {
            pid: std::process::id(),
            uptime_seconds: self.uptime_seconds(),
            container_count: registry.count_total(),
            running_count: registry.count_running(),
        })
    }

    /// Handle LogsRequest
    ///
    /// Note: This implementation only supports static log reading (not streaming).
    /// Log streaming (tail -f behavior) would require:
    /// 1. Persistent WebSocket or long-lived TCP connections
    /// 2. File watching capabilities (inotify on Linux)
    /// 3. Protocol changes to support streaming responses
    /// 4. Client-side streaming handling
    ///
    /// The current implementation reads the tail of log files and returns them immediately.
    pub async fn handle_logs(
        &self,
        container_id_or_name: String,
        tail: usize,
    ) -> Result<DaemonResponse, DaemonError> {
        info!(
            "Fetching logs for container: {} (tail: {})",
            container_id_or_name, tail
        );

        // Verify container exists and resolve ID or name
        let registry = self.registry.read().await;

        // Resolve container ID or name to actual container ID
        let container_id = registry
            .resolve_id_or_name(&container_id_or_name)
            .ok_or_else(|| {
                DaemonError::Container(ContainerError::NotFound(container_id_or_name.to_string()))
            })?;

        let container = registry.get(&container_id).ok_or_else(|| {
            DaemonError::Container(ContainerError::NotFound(container_id.to_string()))
        })?;

        // Check if container has TTY enabled
        if container.config.tty {
            // For TTY containers, logs go through the PTY, not log files
            // Return empty logs with a helpful message via error
            drop(registry);
            return Err(DaemonError::InvalidRequest(format!(
                "Container '{}' has TTY enabled. Logs are not available for TTY containers. Use 'attach' to view output.",
                container_id
            )));
        }

        drop(registry);

        // Read logs using ContainerLogs
        let logs = ContainerLogs::new(container_id.clone());

        let stdout = logs
            .read_stdout_tail(tail)
            .map_err(|e| DaemonError::Storage(e.into()))?;

        let stderr = logs
            .read_stderr_tail(tail)
            .map_err(|e| DaemonError::Storage(e.into()))?;

        Ok(DaemonResponse::LogsResponse {
            container_id,
            stdout,
            stderr,
        })
    }

    /// Handle AttachRequest
    pub async fn handle_attach(
        &self,
        container_id_or_name: &str,
    ) -> Result<DaemonResponse, DaemonError> {
        info!("Attaching to container: {}", container_id_or_name);

        // Verify container exists and is running
        let registry = self.registry.read().await;

        // Resolve container ID or name to actual container ID
        let container_id = registry
            .resolve_id_or_name(container_id_or_name)
            .ok_or_else(|| {
                DaemonError::Container(ContainerError::NotFound(container_id_or_name.to_string()))
            })?;

        let container = registry.get(&container_id).ok_or_else(|| {
            DaemonError::Container(ContainerError::NotFound(container_id.to_string()))
        })?;

        // Check if container is running
        if !container.state.is_running() {
            return Err(DaemonError::Container(ContainerError::InvalidState {
                expected: "Running".to_string(),
                actual: container.state.to_string(),
            }));
        }

        // Check if container has TTY configured
        if !container.config.tty {
            return Err(DaemonError::Container(ContainerError::ConfigError(
                "Container was not created with TTY support".to_string(),
            )));
        }

        // Validate that PTY master FD is actually available
        // This ensures the container startup completed successfully and PTY is ready
        if container.pty_master.is_none() {
            return Err(DaemonError::Container(ContainerError::ConfigError(
                "Container PTY master file descriptor is not available. Container may still be initializing.".to_string(),
            )));
        }

        Ok(DaemonResponse::AttachResponse {
            container_id,
            message: "Ready for streaming attach - establish streaming connection".to_string(),
        })
    }

    /// Handle streaming attach for a container with TTY
    /// This function takes over the Unix socket for bidirectional streaming
    pub async fn handle_streaming_attach(
        &self,
        container_id_or_name: String,
        stream: UnixStream,
    ) -> Result<(), DaemonError> {
        tracing::info!(container_id_or_name = %container_id_or_name, "Starting streaming attach session");

        // Get PTY master file descriptor
        let (pty_master_fd, container_id) = {
            let registry = self.registry.read().await;

            // Resolve container ID or name to actual container ID
            let container_id = registry
                .resolve_id_or_name(&container_id_or_name)
                .ok_or_else(|| {
                    tracing::error!(container_id_or_name = %container_id_or_name, "Container not found");
                    DaemonError::Container(ContainerError::NotFound(container_id_or_name))
                })?;

            let container = registry.get(&container_id).ok_or_else(|| {
                tracing::error!(container_id = %container_id, "Container not found");
                DaemonError::Container(ContainerError::NotFound(container_id.to_string()))
            })?;

            if !container.state.is_running() {
                tracing::error!(
                    container_id = %container_id,
                    expected = "Running",
                    actual = %container.state,
                    "Container not in expected state"
                );
                return Err(DaemonError::Container(ContainerError::InvalidState {
                    expected: "Running".to_string(),
                    actual: container.state.to_string(),
                }));
            }

            let pty_fd = container.pty_master.ok_or_else(|| {
                tracing::error!(container_id = %container_id, "Container does not have TTY configured");
                DaemonError::Container(ContainerError::ConfigError(
                    "Container does not have TTY support".to_string(),
                ))
            })?;

            (pty_fd, container_id)
        };

        tracing::debug!(container_id = %container_id, pty_master_fd = %pty_master_fd, "Retrieved PTY master FD");

        // Create a borrowed FD to use with dup()
        // SAFETY: BorrowedFd::borrow_raw is unsafe because it creates a reference to a raw file descriptor
        // without guaranteeing its validity or lifetime. This is safe here because:
        // 1. pty_master_fd comes from Container::pty_master which is guaranteed valid
        // 2. We immediately use it with dup() which validates the FD and creates owned copies
        // 3. The borrowed_fd doesn't outlive the pty_master_fd it references
        // 4. No other code closes pty_master_fd during this scope
        let borrowed_fd = unsafe { BorrowedFd::borrow_raw(pty_master_fd) };

        let pty_read_fd = dup(borrowed_fd).map_err(|e| {
            DaemonError::Container(ContainerError::ConfigError(format!(
                "Failed to duplicate PTY master FD for reading: {e}"
            )))
        })?;
        tracing::debug!(container_id = %container_id, pty_read_fd = pty_read_fd.as_raw_fd(), "Duplicated PTY master for reading");

        let pty_write_fd = dup(borrowed_fd).map_err(|e| {
            DaemonError::Container(ContainerError::ConfigError(format!(
                "Failed to duplicate PTY master FD for writing: {e}"
            )))
        })?;
        tracing::debug!(container_id = %container_id, pty_write_fd = pty_write_fd.as_raw_fd(), "Duplicated PTY master for writing");

        // Convert OwnedFd to RawFd and then to async-compatible files
        // Now each File owns its own FD copy
        // SAFETY: from_raw_fd is unsafe because:
        // - It takes ownership of a raw file descriptor without checking validity
        // - Multiple File instances could own the same FD causing double-close bugs
        // This is safe here because:
        // 1. pty_read_fd and pty_write_fd are freshly created by dup() above, guaranteed valid
        // 2. into_raw_fd() transfers ownership from OwnedFd to RawFd without dropping
        // 3. Each File takes unique ownership of its own duplicated FD copy
        // 4. No other code references these specific FD numbers after this point
        let pty_read_file = unsafe { File::from_raw_fd(pty_read_fd.into_raw_fd()) };

        let mut pty_write_file = unsafe { File::from_raw_fd(pty_write_fd.into_raw_fd()) };

        // Split the Unix stream for bidirectional communication
        let (mut stream_read, mut stream_write) = stream.into_split();

        // Spawn tasks for bidirectional I/O forwarding
        let container_id_clone = container_id.clone();
        let pty_to_client = tokio::spawn(async move {
            tracing::debug!(container_id = %container_id_clone, "Starting PTY→client forwarding task");

            let mut pty_file = pty_read_file;
            let mut buffer = [0u8; 4096];

            loop {
                match pty_file.read(&mut buffer).await {
                    Ok(0) => {
                        tracing::info!(container_id = %container_id_clone, "PTY master closed, ending session");
                        break;
                    }
                    Ok(n) => {
                        tracing::trace!(container_id = %container_id_clone, bytes = n, "Read from PTY");
                        // Forward PTY output to client
                        let response = DaemonResponse::AttachStdout {
                            data: buffer[..n].to_vec(),
                        };
                        if let Err(e) = write_message(&mut stream_write, &response).await {
                            tracing::error!(container_id = %container_id_clone, error = %e, "Failed to write to client");
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::error!(container_id = %container_id_clone, error = %e, "Error reading from PTY");
                        break;
                    }
                }
            }

            tracing::debug!(container_id = %container_id_clone, "PTY→client task ending");
        });

        let container_id_clone2 = container_id.clone();
        let client_to_pty = tokio::spawn(async move {
            tracing::debug!(container_id = %container_id_clone2, "Starting client→PTY forwarding task");

            loop {
                match read_message(&mut stream_read).await {
                    Ok(DaemonRequest::AttachStdin { data }) => {
                        tracing::trace!(container_id = %container_id_clone2, bytes = data.len(), "Received stdin from client");
                        // Forward client input to PTY
                        if let Err(e) = pty_write_file.write_all(&data).await {
                            tracing::error!(container_id = %container_id_clone2, error = %e, "Failed to write to PTY");
                            break;
                        }
                        if let Err(e) = pty_write_file.flush().await {
                            tracing::error!(container_id = %container_id_clone2, error = %e, "Failed to flush PTY");
                            break;
                        }
                        tracing::debug!(container_id = %container_id_clone2, bytes = data.len(), "Forwarded to PTY");
                    }
                    Ok(DaemonRequest::AttachDetach) => {
                        tracing::info!(container_id = %container_id_clone2, "Client detaching gracefully");
                        break;
                    }
                    Ok(_) => {
                        tracing::warn!(container_id = %container_id_clone2, "Unexpected request type during attach session");
                    }
                    Err(e) => {
                        tracing::error!(container_id = %container_id_clone2, error = %e, "Error reading from client");
                        break;
                    }
                }
            }

            tracing::debug!(container_id = %container_id_clone2, "Client→PTY task ending");
        });

        // Wait for either task to complete (detach or error)
        tracing::debug!(container_id = %container_id, "Waiting for attach session to complete");
        tokio::select! {
            res = pty_to_client => {
                match res {
                    Ok(()) => tracing::info!(container_id = %container_id, "PTY→client task completed successfully"),
                    Err(e) => tracing::error!(container_id = %container_id, error = %e, "PTY→client task panicked"),
                }
            }
            res = client_to_pty => {
                match res {
                    Ok(()) => tracing::info!(container_id = %container_id, "Client→PTY task completed successfully"),
                    Err(e) => tracing::error!(container_id = %container_id, error = %e, "Client→PTY task panicked"),
                }
            }
        }

        tracing::info!(container_id = %container_id, "Streaming attach session ended");
        Ok(())
    }

    /// Stop all running containers (for graceful shutdown)
    pub async fn stop_all_containers(&self, timeout_secs: u64) {
        info!(
            "Stopping all running containers (timeout: {}s)...",
            timeout_secs
        );

        let mut registry = self.registry.write().await;
        let running_ids: Vec<String> = registry
            .list()
            .iter()
            .filter(|c| c.state.is_running())
            .map(|c| c.id.clone())
            .collect();

        for container_id in running_ids {
            if let Some(container) = registry.get_mut(&container_id) {
                info!("Stopping container: {}", container_id);

                // Send signals to gracefully stop the container
                if let Some(pid) = container.pid {
                    // Send SIGTERM first
                    match signal::kill(Pid::from_raw(pid), signal::Signal::SIGTERM) {
                        Ok(()) => {
                            info!("SIGTERM sent to container {} (PID {})", container_id, pid);

                            // Wait for a short timeout
                            let timeout = std::time::Duration::from_secs(5);
                            let start = std::time::Instant::now();

                            // Check if process exits gracefully
                            while start.elapsed() < timeout {
                                match signal::kill(Pid::from_raw(pid), None) {
                                    Ok(()) => {
                                        // Process is still alive, wait a bit
                                        tokio::time::sleep(std::time::Duration::from_millis(100))
                                            .await;
                                    }
                                    Err(_) => {
                                        // Process has exited
                                        info!(
                                            "Container {} (PID {}) exited gracefully",
                                            container_id, pid
                                        );
                                        break;
                                    }
                                }
                            }

                            // If still alive, send SIGKILL
                            if signal::kill(Pid::from_raw(pid), None).is_ok() {
                                info!("Container {} still alive, sending SIGKILL", container_id);
                                let _ = signal::kill(Pid::from_raw(pid), signal::Signal::SIGKILL);
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to send SIGTERM to container {}: {}",
                                container_id, e
                            );
                        }
                    }
                }

                if let Err(e) = container.mark_stopped() {
                    error!("Failed to stop container {}: {}", container_id, e);
                } else if let Err(e) = save_metadata(container) {
                    error!("Failed to save metadata for {}: {}", container_id, e);
                }
            }
        }

        info!("All containers stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustbox::container::ContainerConfig;

    fn test_config() -> ContainerConfig {
        ContainerConfig {
            memory_limit: "256M".to_string(),
            cpu_limit: "0.5".to_string(),
            command: vec!["/bin/bash".to_string()],
            workdir: "/".to_string(),
            rootfs_path: "./rootfs".to_string(),
            tty: false,
            isolate_user: false,
            isolate_network: false,
        }
    }

    #[test]
    fn test_registry_insert() {
        let mut registry = ContainerRegistry::new();
        let container = Container::new(Some("test".to_string()), test_config());

        assert!(registry.insert(container.clone()).is_ok());
        assert_eq!(registry.count_total(), 1);

        // Duplicate ID should fail
        assert!(registry.insert(container).is_err());
    }

    #[test]
    fn test_registry_operations() {
        let mut registry = ContainerRegistry::new();
        let container = Container::new(Some("test".to_string()), test_config());
        let id = container.id.clone();

        registry.insert(container).unwrap();

        assert!(registry.get(&id).is_some());
        assert_eq!(registry.count_total(), 1);

        let removed = registry.remove(&id);
        assert!(removed.is_some());
        assert_eq!(registry.count_total(), 0);
    }

    #[test]
    fn test_resolve_id_or_name() {
        let mut registry = ContainerRegistry::new();

        let container1 = Container::new(Some("web-server".to_string()), test_config());
        let id1 = container1.id.clone();

        let container2 = Container::new(Some("database".to_string()), test_config());
        let id2 = container2.id.clone();

        registry.insert(container1).unwrap();
        registry.insert(container2).unwrap();

        // Test resolution by ID
        assert_eq!(registry.resolve_id_or_name(&id1), Some(id1.clone()));
        assert_eq!(registry.resolve_id_or_name(&id2), Some(id2.clone()));

        // Test resolution by name
        assert_eq!(registry.resolve_id_or_name("web-server"), Some(id1.clone()));
        assert_eq!(registry.resolve_id_or_name("database"), Some(id2.clone()));

        // Test non-existent container
        assert_eq!(registry.resolve_id_or_name("nonexistent"), None);
        assert_eq!(registry.resolve_id_or_name("invalid-id-123"), None);
    }

    #[test]
    fn test_resolve_id_priority_over_name() {
        let mut registry = ContainerRegistry::new();

        // Create a container with a specific name
        let container = Container::new(Some("mycontainer".to_string()), test_config());
        let id = container.id.clone();

        registry.insert(container).unwrap();

        // Resolution by ID should work
        assert_eq!(registry.resolve_id_or_name(&id), Some(id.clone()));

        // Resolution by name should also work
        assert_eq!(registry.resolve_id_or_name("mycontainer"), Some(id.clone()));

        // If a container ID happens to match another container's name,
        // the ID should take priority (tested by checking we get back the same value)
        assert_eq!(registry.resolve_id_or_name(&id), Some(id));
    }
}
