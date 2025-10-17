//! Container management and isolation primitives.
//!
//! This module provides the core container abstractions, state management,
//! and isolation mechanisms used by RustBox.
//!
//! ## Key Components
//!
//! - [`Container`]: Core container data structure with state and configuration
//! - [`ContainerState`]: State machine for container lifecycle management
//! - [`ContainerConfig`]: Runtime configuration and resource limits
//! - [`SandboxConfig`]: Low-level sandbox configuration for process isolation
//! - [`OverlayPaths`]: Filesystem overlay management
//!
//! ## Container Lifecycle
//!
//! ```text
//! Created ──start──► Running ──exit──► Exited
//!             │         │
//!             │         └──stop──► Stopped ──timeout──► Exited
//!             │
//!             └──error──► Exited
//! ```
//!
//! ## Example
//!
//! ```rust
//! use rustbox::container::{Container, ContainerConfig};
//!
//! let config = ContainerConfig {
//!     memory_limit: "256M".to_string(),
//!     cpu_limit: "0.5".to_string(),
//!     command: vec!["/bin/bash".to_string()],
//!     workdir: "/".to_string(),
//!     rootfs_path: "./rootfs".to_string(),
//!     tty: false,
//! };
//!
//! let container = Container::new(Some("myapp".to_string()), config);
//! tracing::info!("Created container: {} ({})", container.id, container.name);
//! ```

pub mod config;
pub mod id;
pub mod sandbox;
pub mod state_machine;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use config::{ContainerConfig, OverlayPaths};
pub use id::{generate_container_id, generate_container_name, validate_container_id};
pub use sandbox::{
    drain_cgroup_and_remove, run_sandbox, wait_and_cleanup, SandboxCleanupPaths, SandboxConfig,
    SandboxResult,
};

pub use state_machine::ContainerState;

/// Represents a container's complete state and configuration.
///
/// A `Container` encapsulates all information needed to manage a container's
/// lifecycle, including its current state, resource configuration, and
/// filesystem isolation details.
///
/// ## Fields
///
/// - `id`: Unique 12-character hexadecimal identifier
/// - `name`: Human-readable name (user-provided or auto-generated)
/// - `state`: Current lifecycle state (Created, Running, Stopped, Exited)
/// - `config`: Runtime configuration including resource limits
/// - `overlay_paths`: Filesystem overlay directories
/// - `cgroup_path`: Cgroup directory path for resource isolation
/// - `pid`: Process ID when running (None when not running)
///
/// ## Example
///
/// ```rust
/// use rustbox::container::{Container, ContainerConfig};
///
/// let config = ContainerConfig {
///     memory_limit: "512M".to_string(),
///     cpu_limit: "1.0".to_string(),
///     command: vec!["/bin/bash".to_string()],
///     workdir: "/root".to_string(),
///     rootfs_path: "./rootfs".to_string(),
///     tty: false,
/// };
///
/// let container = Container::new(Some("web-server".to_string()), config);
/// assert_eq!(container.name, "web-server");
/// assert!(container.id.len() == 12);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Container {
    /// Unique 12-character hexadecimal identifier
    pub id: String,

    /// Human-readable name (user-provided or auto-generated)
    pub name: String,

    /// Current lifecycle state
    pub state: ContainerState,

    /// Container creation timestamp
    pub created_at: DateTime<Utc>,

    /// Container start timestamp (None if never started)
    pub started_at: Option<DateTime<Utc>>,

    /// Container finish timestamp (None if still running)
    pub finished_at: Option<DateTime<Utc>>,

    /// Exit code (None if still running, Some(code) if exited)
    pub exit_code: Option<i32>,

    /// Container runtime configuration
    pub config: ContainerConfig,

    /// Filesystem paths for overlay mounting
    pub overlay_paths: OverlayPaths,

    /// Cgroup path for resource isolation
    pub cgroup_path: PathBuf,

    /// Process ID of container init process (None if not running)
    pub pid: Option<i32>,

    /// PTY master file descriptor for containers with TTY (None for non-TTY containers)
    pub pty_master: Option<std::os::unix::io::RawFd>,
}

impl Container {
    /// Creates a new container with the given configuration.
    ///
    /// The container starts in the `Created` state and is assigned a unique
    /// 12-character hexadecimal ID. If no name is provided, an auto-generated
    /// name following the "adjective-noun" pattern is used.
    ///
    /// # Arguments
    ///
    /// * `name` - Optional human-readable name for the container
    /// * `config` - Runtime configuration including resource limits and command
    ///
    /// # Returns
    ///
    /// A new `Container` instance in the `Created` state
    ///
    /// # Example
    ///
    /// ```rust
    /// use rustbox::container::{Container, ContainerConfig};
    ///
    /// let config = ContainerConfig {
    ///     memory_limit: "256M".to_string(),
    ///     cpu_limit: "0.5".to_string(),
    ///     command: vec!["/bin/bash".to_string()],
    ///     workdir: "/".to_string(),
    ///     rootfs_path: "./rootfs".to_string(),
    ///     tty: false,
    /// };
    ///
    /// // Create with explicit name
    /// let container1 = Container::new(Some("myapp".to_string()), config.clone());
    /// assert_eq!(container1.name, "myapp");
    ///
    /// // Create with auto-generated name
    /// let container2 = Container::new(None, config);
    /// tracing::info!("Auto-generated name: {}", container2.name); // e.g., "happy-elephant"
    /// ```
    pub fn new(name: Option<String>, config: ContainerConfig) -> Self {
        let id = generate_container_id();
        let name = name.unwrap_or_else(generate_container_name);
        let overlay_paths = OverlayPaths::new(&id, &config.rootfs_path);
        let cgroup_path = PathBuf::from(crate::constants::CGROUP_BASE)
            .join(crate::constants::CGROUP_NAMESPACE)
            .join(&id);

        Container {
            id,
            name,
            state: ContainerState::Created,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            exit_code: None,
            config,
            overlay_paths,
            cgroup_path,
            pid: None,
            pty_master: None,
        }
    }

    /// Transitions the container to the `Running` state and records the start time.
    ///
    /// This method should be called when a container process is successfully started.
    /// It validates that the state transition from `Created` to `Running` is valid.
    ///
    /// # Arguments
    ///
    /// * `pid` - Process ID of the container's main process
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If the transition was successful
    /// * `Err(String)` - If the current state doesn't allow transitioning to `Running`
    ///
    /// # Example
    ///
    /// ```rust
    /// use rustbox::container::{Container, ContainerConfig, ContainerState};
    ///
    /// let config = ContainerConfig {
    ///     memory_limit: "256M".to_string(),
    ///     cpu_limit: "0.5".to_string(),
    ///     command: vec!["/bin/bash".to_string()],
    ///     workdir: "/".to_string(),
    ///     rootfs_path: "./rootfs".to_string(),
    ///     tty: false,
    /// };
    ///
    /// let mut container = Container::new(None, config);
    /// assert_eq!(container.state, ContainerState::Created);
    ///
    /// container.mark_started(1234).expect("Failed to start container");
    /// assert_eq!(container.state, ContainerState::Running);
    /// assert_eq!(container.pid, Some(1234));
    /// assert!(container.started_at.is_some());
    /// ```
    pub fn mark_started(&mut self, pid: i32) -> Result<(), String> {
        self.state.transition(ContainerState::Running)?;
        self.started_at = Some(Utc::now());
        self.pid = Some(pid);
        Ok(())
    }

    /// Transitions the container to the `Stopped` state.
    ///
    /// This method should be called when a container is stopped by user request
    /// (e.g., via the `rustbox stop` command). It validates that the state transition
    /// from `Running` to `Stopped` is valid and records the finish timestamp.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - If the transition was successful
    /// * `Err(String)` - If the current state doesn't allow transitioning to `Stopped`
    ///
    /// # Example
    ///
    /// ```rust
    /// use rustbox::container::{Container, ContainerConfig, ContainerState};
    ///
    /// let config = ContainerConfig {
    ///     memory_limit: "256M".to_string(),
    ///     cpu_limit: "0.5".to_string(),
    ///     command: vec!["/bin/bash".to_string()],
    ///     workdir: "/".to_string(),
    ///     rootfs_path: "./rootfs".to_string(),
    ///     tty: false,
    /// };
    ///
    /// let mut container = Container::new(None, config);
    /// container.mark_started(1234).expect("Failed to start");
    /// assert_eq!(container.state, ContainerState::Running);
    ///
    /// container.mark_stopped().expect("Failed to stop container");
    /// assert_eq!(container.state, ContainerState::Stopped);
    /// assert!(container.finished_at.is_some());
    /// ```
    pub fn mark_stopped(&mut self) -> Result<(), String> {
        self.state.transition(ContainerState::Stopped)?;
        self.finished_at = Some(Utc::now());
        Ok(())
    }

    /// Mark container as exited with an exit code
    pub fn mark_exited(&mut self, exit_code: i32) -> Result<(), String> {
        self.state.transition(ContainerState::Exited)?;
        self.finished_at = Some(Utc::now());
        self.exit_code = Some(exit_code);
        self.pid = None;
        Ok(())
    }

    /// Get container uptime in seconds (if running)
    pub fn uptime_seconds(&self) -> Option<i64> {
        if let Some(started_at) = self.started_at {
            if self.state.is_running() {
                Some((Utc::now() - started_at).num_seconds())
            } else { self.finished_at.map(|finished_at| (finished_at - started_at).num_seconds()) }
        } else {
            None
        }
    }
}

/// Implements proper cleanup for Container resources.
///
/// When a Container is dropped, this ensures the PTY master file descriptor
/// is properly closed if it exists, preventing FD leaks.
impl Drop for Container {
    fn drop(&mut self) {
        if let Some(fd) = self.pty_master {
            // SAFETY: We own this RawFd - it was transferred to us via into_raw_fd()
            // in sandbox.rs. Closing it here prevents FD leaks. This is safe because:
            // 1. The FD is valid (created by openpty and transferred via into_raw_fd)
            // 2. We only close it once (pty_master is consumed after this)
            // 3. No other code holds references to this specific FD value
            use nix::unistd::close;
            if let Err(e) = close(fd) {
                // Log error but don't panic - we're in a destructor
                tracing::error!("Warning: Failed to close PTY master FD {fd}: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ContainerConfig {
        ContainerConfig {
            memory_limit: "256M".to_string(),
            cpu_limit: "0.5".to_string(),
            command: vec!["/bin/bash".to_string()],
            workdir: "/".to_string(),
            rootfs_path: "./rootfs".to_string(),
            tty: false,
        }
    }

    #[test]
    fn test_container_new() {
        let container = Container::new(Some("myapp".to_string()), test_config());
        assert_eq!(container.name, "myapp");
        assert_eq!(container.state, ContainerState::Created);
        assert!(container.started_at.is_none());
        assert!(container.pid.is_none());
    }

    #[test]
    fn test_container_lifecycle() {
        let mut container = Container::new(None, test_config());
        assert_eq!(container.state, ContainerState::Created);

        // Start container
        assert!(container.mark_started(1234).is_ok());
        assert_eq!(container.state, ContainerState::Running);
        assert_eq!(container.pid, Some(1234));
        assert!(container.started_at.is_some());

        // Stop container
        assert!(container.mark_stopped().is_ok());
        assert_eq!(container.state, ContainerState::Stopped);

        // Exit container
        assert!(container.mark_exited(0).is_ok());
        assert_eq!(container.state, ContainerState::Exited);
        assert_eq!(container.exit_code, Some(0));
        assert!(container.pid.is_none());
    }

    #[test]
    fn test_container_id_generation() {
        let container1 = Container::new(None, test_config());
        let container2 = Container::new(None, test_config());
        assert_ne!(container1.id, container2.id);
        assert_eq!(container1.id.len(), 12);
        assert_eq!(container2.id.len(), 12);
    }
}
