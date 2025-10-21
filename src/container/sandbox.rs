use crate::constants::CGROUP_BASE;
use nix::{
    mount::{mount, umount2, MntFlags, MsFlags},
    pty::openpty,
    sched::{unshare, CloneFlags},
    sys::wait::waitpid,
    unistd::{chdir, chroot, close, execv, fork, ForkResult},
};
use std::{
    ffi::CString,
    fs::{create_dir_all, read_dir, read_to_string, remove_dir, write, OpenOptions},
    os::unix::io::{IntoRawFd, RawFd},
    path::{Path, PathBuf},
    process,
};
use std::{fs::symlink_metadata, io::Write};
use tracing::{error, info, warn};

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub lower_dir: PathBuf,
    pub upper_dir: PathBuf,
    pub work_dir: PathBuf,
    pub merged_dir: PathBuf,
    pub memory_limit: String,
    pub command: Vec<String>,
    pub workdir: String,
    pub cpu_limit: String,
    pub stdout_log_path: Option<String>,
    pub stderr_log_path: Option<String>,
    pub tty: bool,
}

/// Move any remaining PIDs in cgroup back to root cgroup before removing directory.
pub fn drain_cgroup_and_remove(cgroup_path: &Path) {
    let procs_file = cgroup_path.join("cgroup.procs");
    if let Ok(contents) = read_to_string(&procs_file) {
        for pid_line in contents.lines().map(str::trim).filter(|s| !s.is_empty()) {
            if let Err(e) = write("/sys/fs/cgroup/cgroup.procs", pid_line) {
                warn!(
                    "Failed to move pid {} out of {}: {}",
                    pid_line,
                    cgroup_path.display(),
                    e
                );
            } else {
                info!("Moved pid {} out of {}", pid_line, cgroup_path.display());
            }
        }
    }

    match remove_dir(cgroup_path) {
        Ok(_) => info!("Successfully removed cgroup: {}", cgroup_path.display()),
        Err(e) => warn!("Failed to remove cgroup {}: {}", cgroup_path.display(), e),
    }
}

fn parse_cpu_limit(cpu_limit_str: &str) -> Result<String, String> {
    let cpu_fraction: f64 = cpu_limit_str
        .parse()
        .map_err(|e| format!("Invalid CPU limit format '{cpu_limit_str}': {e}"))?;

    if cpu_fraction <= 0.0 {
        return Err("CPU limit must be positive".to_string());
    }

    // cgroup v2 recommended period is 100000 microseconds (100ms)
    let period = 100_000u64;
    let quota = (cpu_fraction * period as f64) as u64;
    Ok(format!("{quota} {period}"))
}

fn generate_cgroup_path() -> PathBuf {
    Path::new(CGROUP_BASE).join(format!("rustbox_{}", process::id()))
}

fn setup_cgroup(config: &SandboxConfig, cgroup_path: &Path, pid: u32) -> Result<(), String> {
    create_dir_all(cgroup_path).map_err(|e| format!("failed to create cgroup path: {e}"))?;

    write(cgroup_path.join("memory.max"), config.memory_limit.clone())
        .map_err(|e| format!("Failed to set memory limit: {e}"))?;

    let cpu_quota = parse_cpu_limit(&config.cpu_limit)?;
    write(cgroup_path.join("cpu.max"), cpu_quota)
        .map_err(|e| format!("Failed to set CPU limit: {e}"))?;

    write(cgroup_path.join("cgroup.procs"), pid.to_string())
        .map_err(|e| format!("Failed to add process to cgroup: {e}"))?;

    Ok(())
}

fn ensure_dirs_exist(dirs: &[&Path]) -> Result<(), String> {
    for d in dirs {
        if !d.exists() {
            create_dir_all(d).map_err(|e| format!("Failed to create {}: {}", d.display(), e))?;
            info!("Created directory: {}", d.display());
        }
    }
    Ok(())
}

pub fn mount_overlay(lower: &Path, upper: &Path, work: &Path, merged: &Path) -> Result<(), String> {
    let opts = format!(
        "lowerdir={},upperdir={},workdir={}",
        lower.display(),
        upper.display(),
        work.display()
    );
    info!(
        "Mounting overlay at {} with opts: {}",
        merged.display(),
        opts
    );
    mount(
        Some("overlay"),
        merged,
        Some("overlay"),
        MsFlags::empty(),
        Some(opts.as_str()),
    )
    .map_err(|e| format!("overlay mount failed: {e}"))?;
    Ok(())
}

#[inline]
pub fn umount_detach(path: &Path) {
    if let Err(e) = umount2(path, MntFlags::MNT_DETACH) {
        warn!("umount2 failed for {}: {}", path.display(), e);
    }
}

fn mount_proc_and_dev(merged: &Path) -> Result<(), String> {
    let proc_path = merged.join("proc");
    info!("Mounting /proc at: {}", proc_path.display());
    mount(
        Some("proc"),
        &proc_path,
        Some("proc"),
        MsFlags::empty(),
        None::<&str>,
    )
    .map_err(|e| format!("mount /proc failed: {e}"))?;

    let dev_path = merged.join("dev");
    info!("Bind-mounting /dev at: {}", dev_path.display());
    mount(
        Some("/dev"),
        &dev_path,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .map_err(|e| format!("mount /dev failed: {e}"))?;

    Ok(())
}

/// Setup PTY redirection for TTY containers
/// Configures stdin/stdout/stderr to use the PTY slave file descriptor
fn setup_pty_redirection(slave_fd: RawFd) -> Result<(), String> {
    // DEBUG: Write to a debug file to see slave_fd value
    if let Ok(mut debug_file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/pty_debug.log")
    {
        let _ = writeln!(debug_file, "[INNER CHILD] slave_fd={}", slave_fd);
        // Check what slave_fd points to
        if let Ok(link) = std::fs::read_link(format!("/proc/self/fd/{}", slave_fd)) {
            let _ = writeln!(
                debug_file,
                "[INNER CHILD] slave_fd {} points to: {:?}",
                slave_fd, link
            );
        }
    }

    // SAFETY: Creating a new session and setting up PTY controlling terminal
    // This is safe because:
    // 1. We're in a freshly forked child process with no other threads
    // 2. setsid() creates a new session, making this process the session leader
    // 3. The PTY slave becomes the controlling terminal for this session
    unsafe {
        // Create a new session - this is required before TIOCSCTTY
        // Without this, the PTY master will return EIO when trying to read/write
        if libc::setsid() < 0 {
            tracing::error!("[ERROR] Inner child: Failed to create new session (setsid)");
            process::exit(1);
        }
        tracing::debug!("[DEBUG] Inner child: setsid() succeeded");
    }

    // SAFETY: Manipulating file descriptors with close and dup2 is unsafe because:
    // - We're directly manipulating OS-level file descriptor numbers
    // - Invalid FD operations could affect other parts of the program
    // This is safe here because:
    // 1. We're in a freshly forked child process with no other threads
    // 2. We close fds 0, 1, 2 (standard fds) before duplicating the PTY slave
    // 3. The PTY slave fd is guaranteed valid (created by openpty earlier)
    // 4. We close the original slave_fd after duplication to prevent leaks
    unsafe {
        close(0).ok(); // Close stdin
        close(1).ok(); // Close stdout
        close(2).ok(); // Close stderr

        // Use libc::dup2 for raw file descriptors and check for errors
        let ret0 = libc::dup2(slave_fd, 0);
        tracing::debug!(
            "[DEBUG] Inner child: dup2({}, 0) returned {}",
            slave_fd,
            ret0
        );
        if ret0 < 0 {
            tracing::error!("[ERROR] Inner child: Failed to dup2 PTY slave to stdin");
            process::exit(1);
        }

        let ret1 = libc::dup2(slave_fd, 1);
        tracing::debug!(
            "[DEBUG] Inner child: dup2({}, 1) returned {}",
            slave_fd,
            ret1
        );
        if ret1 < 0 {
            tracing::error!("[ERROR] Inner child: Failed to dup2 PTY slave to stdout");
            process::exit(1);
        }

        let ret2 = libc::dup2(slave_fd, 2);
        tracing::debug!(
            "[DEBUG] Inner child: dup2({}, 2) returned {}",
            slave_fd,
            ret2
        );
        if ret2 < 0 {
            tracing::error!("[ERROR] Inner child: Failed to dup2 PTY slave to stderr");
            process::exit(1);
        }

        // Set PTY slave as the controlling terminal
        // TIOCSCTTY = "Terminal I/O Control: Set Controlling TTY"
        // This ioctl makes the PTY slave the controlling terminal for this session
        // Without this, the PTY master/slave pair won't work properly for I/O
        let ioctl_ret = libc::ioctl(0, libc::TIOCSCTTY, 0);
        tracing::debug!(
            "[DEBUG] Inner child: ioctl(TIOCSCTTY) returned {}",
            ioctl_ret
        );
        if ioctl_ret < 0 {
            tracing::error!("[ERROR] Inner child: Failed to set controlling terminal");
            process::exit(1);
        }

        // Close the original PTY slave fd since we've duplicated it to 0, 1, 2
        tracing::debug!(
            "[DEBUG] Inner child: Closing original slave_fd={}",
            slave_fd
        );
        close(slave_fd).ok();
        tracing::debug!("[DEBUG] Inner child: PTY setup complete");
    }

    Ok(())
}

/// Setup log file redirection for non-TTY containers
/// Redirects stdin to /dev/null and stdout/stderr to log files
fn setup_log_file_redirection(config: &SandboxConfig) -> Result<(), String> {
    // Open /dev/null before chroot for stdin
    let devnull_read = OpenOptions::new()
        .read(true)
        .open("/dev/null")
        .map_err(|e| format!("Failed to open /dev/null for reading: {e}"))?;

    let null_in_fd = devnull_read.into_raw_fd(); // Take ownership of fd

    // Open log files for stdout/stderr if provided
    let (stdout_fd, stderr_fd) = if let (Some(stdout_path), Some(stderr_path)) =
        (&config.stdout_log_path, &config.stderr_log_path)
    {
        let stdout_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(stdout_path)
            .map_err(|e| format!("Failed to open stdout log {stdout_path}: {e}"))?;

        let stderr_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(stderr_path)
            .map_err(|e| format!("Failed to open stderr log {stderr_path}: {e}"))?;

        // Take ownership of fds to prevent automatic close
        (stdout_file.into_raw_fd(), stderr_file.into_raw_fd())
    } else {
        // Fallback to /dev/null if no log paths provided
        let devnull_write = OpenOptions::new()
            .write(true)
            .open("/dev/null")
            .map_err(|e| format!("Failed to open /dev/null for writing: {e}"))?;
        let fd = devnull_write.into_raw_fd();
        (fd, fd)
    };

    // Close and reopen stdin/stdout/stderr
    // SAFETY: Manipulating file descriptors with close, dup2, and libc::close is unsafe because:
    // - We're directly manipulating OS-level file descriptor numbers
    // - Invalid FD operations could affect other parts of the program
    // This is safe here because:
    // 1. We're in a freshly forked child process with no other threads
    // 2. We close fds 0, 1, 2 (standard fds) before duplicating log file fds
    // 3. The log file fds are guaranteed valid (created via OpenOptions above)
    // 4. We close the original fds after duplication to prevent leaks
    unsafe {
        close(0).ok(); // Close stdin
        close(1).ok(); // Close stdout
        close(2).ok(); // Close stderr

        // Duplicate file descriptors to stdin/stdout/stderr
        libc::dup2(null_in_fd, 0); // stdin -> /dev/null
        libc::dup2(stdout_fd, 1); // stdout -> log file
        libc::dup2(stderr_fd, 2); // stderr -> log file
    }

    Ok(())
}

/// Setup container environment: mounts, chroot, chdir, and ownership
fn setup_container_environment(config: &SandboxConfig, merged: &Path) -> Result<(), String> {
    mount_proc_and_dev(merged)?;

    info!("(inner child) Changing root to: {}", merged.display());
    chroot(merged).map_err(|e| format!("chroot failed: {e}"))?;

    info!(
        "(inner child) Changing working directory to: {}",
        config.workdir
    );
    chdir(config.workdir.as_str()).map_err(|e| format!("chdir failed: {e}"))?;

    // Only do this inside the container, after chroot/chdir
    if let Err(e) = chown_recursive(Path::new("."), 65534, 65534) {
        warn!("Failed to chown workdir recursively to nobody: {}", e);
    } else {
        info!("Successfully chowned workdir to nobody (65534:65534)");
    }

    Ok(())
}

/// Execute the configured command using execv
fn execute_command(config: &SandboxConfig) -> Result<(), String> {
    info!("(inner child) Executing command: {:?}", config.command);

    let program = CString::new(
        config
            .command
            .first()
            .ok_or("Command vector is empty")?
            .clone(),
    )
    .map_err(|e| format!("Invalid program path: {e}"))?;

    // Convert all command arguments to CStrings
    let args: Result<Vec<CString>, _> = config
        .command
        .iter()
        .map(|arg| CString::new(arg.clone()))
        .collect();
    let args = args.map_err(|e| format!("Invalid command argument: {e}"))?;

    execv(&program, &args).map_err(|e| format!("execv failed: {e}"))?;

    unreachable!("execv should not return");
}

/// Handle the inner child process setup and execution
fn handle_inner_child(
    config: &SandboxConfig,
    merged: &Path,
    pty_slave_fd: Option<RawFd>,
) -> Result<(), String> {
    info!(
        "(inner child) Setting up container environment in root: {}",
        merged.display()
    );
    
    // Redirect stdin/stdout/stderr
    // For TTY containers: redirect to PTY slave
    // For non-TTY containers: stdin -> /dev/null, stdout/stderr -> log files
    if let Some(slave_fd) = pty_slave_fd {
        setup_pty_redirection(slave_fd)?;
    } else {
        setup_log_file_redirection(config)?;
    }

    setup_container_environment(config, merged)?;
    execute_command(config)?;

    Ok(())
}

/// Handle the inner parent process: wait for child and cleanup mounts
fn handle_inner_parent(
    child: nix::unistd::Pid,
    merged: &Path,
    pty_slave_fd: Option<RawFd>,
) -> Result<(), String> {
    // Close PTY slave in the inner parent - only the inner child needs it
    // If we don't close it here, the namespaced parent keeps it open,
    // which can cause EIO when trying to read from the PTY master
    if let Some(slave_fd) = pty_slave_fd {
        nix::unistd::close(slave_fd).ok();
    }

    info!("(inner parent) Waiting for inner child pid {}...", child);
    let _ = waitpid(child, None);

    info!("(inner parent) Cleaning up mounts inside namespace...");
    umount_detach(&merged.join("proc"));
    umount_detach(&merged.join("dev"));

    Ok(())
}

fn run_in_namespace_and_wait(
    config: &SandboxConfig,
    merged: &Path,
    pty_slave_fd: Option<RawFd>,
) -> Result<(), String> {
    if let Some(fd) = pty_slave_fd {
        info!("(namespaced parent) Received PTY slave_fd={}", fd);
    }

    info!("Creating namespaces...");
    unshare(
        CloneFlags::CLONE_NEWNS
            | CloneFlags::CLONE_NEWPID
            | CloneFlags::CLONE_NEWUTS
            | CloneFlags::CLONE_NEWIPC
            | CloneFlags::CLONE_NEWNET
            | CloneFlags::CLONE_NEWUSER,
    )
    .map_err(|e| format!("unshare failed: {e}"))?;

    info!("Namespaces created successfully, forking inner process...");

    // 1. We call fork() early in the process lifecycle before spawning additional threads
    // 2. The child process immediately calls exec (execve) which replaces the process image
    // 3. We don't hold any locks or use thread-local storage across the fork boundary
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            handle_inner_child(config, merged, pty_slave_fd)?;
            unreachable!("handle_inner_child should not return");
        }
        Ok(ForkResult::Parent { child, .. }) => handle_inner_parent(child, merged, pty_slave_fd),
        Err(e) => Err(format!("inner fork failed: {e}")),
    }
}

// Recursively chown workdir to nobody (UID/GID 65534)
fn chown_recursive(path: &Path, uid: u32, gid: u32) -> std::io::Result<()> {
    let meta = symlink_metadata(path)?;
    if let Some(p) = path.to_str() {
        unsafe {
            libc::chown(p.as_ptr() as *const i8, uid, gid);
        }
    }
    if meta.is_dir() {
        for entry in read_dir(path)? {
            let entry = entry?;
            chown_recursive(&entry.path(), uid, gid)?;
        }
    }
    Ok(())
}

/// Result of starting a sandbox, containing PTY master FD and child PID
pub struct SandboxResult {
    pub pty_master: Option<RawFd>,
    pub child_pid: nix::unistd::Pid,
    pub cleanup_paths: SandboxCleanupPaths,
}

#[derive(Debug, Clone)]
pub struct SandboxCleanupPaths {
    pub merged: PathBuf,
    pub cgroup: PathBuf,
}

pub fn run_sandbox(config: SandboxConfig) -> Result<SandboxResult, String> {
    info!("Starting run_sandbox with config: {:?}", config);

    // Use the overlay paths directly from config
    let lower = &config.lower_dir;
    let upper = &config.upper_dir;
    let work = &config.work_dir;
    let merged = &config.merged_dir;

    ensure_dirs_exist(&[lower, upper, work, merged])?;

    mount_overlay(&lower, &upper, &work, &merged)?;

    let cgroup_path = generate_cgroup_path();
    info!("Generated cgroup path: {}", cgroup_path.display());

    // Create PTY if TTY is requested
    let pty_result = if config.tty {
        match openpty(None, None) {
            Ok(pty) => {
                info!("Created PTY master/slave pair, {:?}", pty);
                Some(pty)
            }
            Err(e) => {
                return Err(format!("Failed to create PTY: {e}"));
            }
        }
    } else {
        None
    };

    // SAFETY: Transfer ownership of PTY master and slave FDs to prevent automatic closure.
    // Using into_raw_fd() consumes both PtyMaster and PtySlave, preventing their Drop
    // implementations from closing the file descriptors when pty_result goes out of scope.
    // The master FD will be stored in Container::pty_master and must be manually closed
    // via nix::unistd::close() when the container is destroyed.
    // The slave FD is passed to the child process and must be closed after dup2.
    //
    // We extract both master FD and slave FD here before pty_result is consumed.
    let (pty_master, pty_slave) = match pty_result {
        Some(pty) => {
            let master_fd = pty.master.into_raw_fd();
            let slave_fd = pty.slave.into_raw_fd(); // Prevent slave from being auto-closed!
            info!(
                "Extracted PTY master_fd={}, slave_fd={}",
                master_fd, slave_fd
            );
            (Some(master_fd), Some(slave_fd))
        }
        None => (None, None),
    };

    // SAFETY: fork() is unsafe because it can cause undefined behavior in multi-threaded programs.
    // This is safe here because:
    // 1. This is the first fork (before the inner child fork), called early in the process
    // 2. We're in a namespace context but haven't spawned threads yet
    // 3. The child process continues with container setup and calls another fork, never returning to shared state
    // 4. The parent returns immediately with the child PID (non-blocking)
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            // Close PTY master in child to prevent keeping it open
            if let Some(master_fd) = pty_master {
                nix::unistd::close(master_fd).ok();
            }

            info!(
                "(namespaced parent) Setting up cgroups at: {}",
                cgroup_path.display()
            );
            if let Err(e) = setup_cgroup(&config, &cgroup_path, process::id()) {
                error!("(namespaced parent) Failed to setup cgroup: {}", e);
                process::exit(1);
            }

            info!("(namespaced parent) Calling unshare & inner fork...");
            match run_in_namespace_and_wait(&config, &merged, pty_slave) {
                Ok(_) => {
                    info!("(namespaced parent) Inner child finished, exiting namespaced parent.");
                    process::exit(0);
                }
                Err(e) => {
                    error!("(namespaced parent) Error: {}", e);
                    process::exit(1);
                }
            }
        }
        Ok(ForkResult::Parent { child, .. }) => {
            info!(
                "(outer parent) Forked namespaced parent with pid {}, returning immediately",
                child
            );

            // Return immediately with PTY master FD and child PID
            // The daemon will wait for the child and handle cleanup
            Ok(SandboxResult {
                pty_master,
                child_pid: child,
                cleanup_paths: SandboxCleanupPaths {
                    merged: merged.clone(),
                    cgroup: cgroup_path,
                },
            })
        }
        Err(e) => Err(format!("initial fork failed: {e}")),
    }
}

/// Wait for a container process to exit and perform cleanup
pub fn wait_and_cleanup(sandbox_result: SandboxResult) -> Result<(), String> {
    info!(
        "(outer parent) Waiting for namespaced parent pid {}...",
        sandbox_result.child_pid
    );
    let _ = waitpid(sandbox_result.child_pid, None);

    info!(
        "(outer parent) Cleaning up overlay mount at: {}",
        sandbox_result.cleanup_paths.merged.display()
    );
    umount_detach(&sandbox_result.cleanup_paths.merged);

    info!(
        "(outer parent) Draining and removing cgroup at: {}",
        sandbox_result.cleanup_paths.cgroup.display()
    );
    drain_cgroup_and_remove(&sandbox_result.cleanup_paths.cgroup);

    info!("Container cleanup completed.");
    Ok(())
}
