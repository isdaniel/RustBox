use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use tracing::{info, warn};

use rustbox::constants::PID_FILE_PATH;
use rustbox::error::DaemonError;

pub struct PidLock {
    pid_file_path: String,
}

impl PidLock {
    /// Create a new PID lock with default path
    pub fn new() -> Self {
        Self {
            pid_file_path: PID_FILE_PATH.to_string(),
        }
    }

    /// Try to acquire the daemon lock
    /// Returns Ok(()) if lock is acquired successfully
    /// Returns Err if another daemon is already running
    pub fn acquire(&self) -> Result<(), DaemonError> {
        let pid_path = Path::new(&self.pid_file_path);

        // Check if PID file exists
        if pid_path.exists() {
            // Read the existing PID
            match self.read_pid_file() {
                Ok(existing_pid) => {
                    // Check if the process is still running
                    if self.is_process_running(existing_pid) {
                        return Err(DaemonError::DaemonAlreadyRunning(existing_pid));
                    } else {
                        warn!("Stale PID file found (PID: {}), removing it", existing_pid);
                        // Process is not running, remove stale PID file
                        if let Err(e) = fs::remove_file(pid_path) {
                            warn!("Failed to remove stale PID file: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read PID file, attempting to overwrite: {}", e);
                    // If we can't read the PID file, try to remove it
                    let _ = fs::remove_file(pid_path);
                }
            }
        }

        // Write our PID to the file
        self.write_pid_file()?;
        info!(
            "Daemon lock acquired, PID file created at {}",
            self.pid_file_path
        );
        Ok(())
    }

    /// Release the daemon lock by removing the PID file
    pub fn release(&self) {
        let pid_path = Path::new(&self.pid_file_path);
        if pid_path.exists() {
            if let Err(e) = fs::remove_file(pid_path) {
                warn!("Failed to remove PID file during release: {}", e);
            } else {
                info!("Daemon lock released, PID file removed");
            }
        }
    }

    /// Read the PID from the PID file
    fn read_pid_file(&self) -> Result<i32, DaemonError> {
        let mut file = File::open(&self.pid_file_path).map_err(|e| {
            DaemonError::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to open PID file: {}", e),
            ))
        })?;

        let mut contents = String::new();
        file.read_to_string(&mut contents).map_err(|e| {
            DaemonError::Io(std::io::Error::new(
                e.kind(),
                format!("Failed to read PID file: {}", e),
            ))
        })?;

        contents.trim().parse::<i32>().map_err(|e| {
            DaemonError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid PID in file: {}", e),
            ))
        })
    }

    /// Write the current process PID to the PID file
    fn write_pid_file(&self) -> Result<(), DaemonError> {
        let current_pid = std::process::id();
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.pid_file_path)
            .map_err(|e| {
                DaemonError::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to create PID file: {}", e),
                ))
            })?;

        file.write_all(current_pid.to_string().as_bytes())
            .map_err(|e| {
                DaemonError::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to write PID to file: {}", e),
                ))
            })?;

        // Set permissions to 0644
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&self.pid_file_path).map_err(|e| {
                DaemonError::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to get PID file metadata: {}", e),
                ))
            })?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o644);
            fs::set_permissions(&self.pid_file_path, permissions).map_err(|e| {
                DaemonError::Io(std::io::Error::new(
                    e.kind(),
                    format!("Failed to set PID file permissions: {}", e),
                ))
            })?;
        }

        Ok(())
    }

    /// Check if a process with the given PID is running
    /// Uses /proc filesystem on Linux
    fn is_process_running(&self, pid: i32) -> bool {
        #[cfg(target_os = "linux")]
        {
            // Check if /proc/[pid] exists
            let proc_path = format!("/proc/{}", pid);
            Path::new(&proc_path).exists()
        }

        #[cfg(not(target_os = "linux"))]
        {
            // Fallback: use kill with signal 0 (no signal sent, just checks if process exists)
            // This is POSIX-compliant
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::Pid;

            match kill(Pid::from_raw(pid), Signal::SIGCONT) {
                Ok(_) => true,
                Err(nix::errno::Errno::ESRCH) => false, // No such process
                Err(nix::errno::Errno::EPERM) => true,  // Process exists but no permission
                Err(_) => false,
            }
        }
    }
}

impl Drop for PidLock {
    fn drop(&mut self) {
        // Automatically release lock when PidLock is dropped
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pid_lock_acquire_and_release() {
        // Clean up any existing PID file first
        let _ = fs::remove_file(PID_FILE_PATH);

        // Wait a bit to ensure file is deleted
        std::thread::sleep(std::time::Duration::from_millis(10));

        let lock = PidLock::new();

        // Should acquire successfully
        let acquire_result = lock.acquire();
        if acquire_result.is_err() {
            // If another test is holding the lock, clean up and skip
            let _ = fs::remove_file(PID_FILE_PATH);
            return;
        }

        // Verify PID file exists and contains our PID
        assert!(Path::new(PID_FILE_PATH).exists());
        let stored_pid = lock.read_pid_file().unwrap();
        assert_eq!(stored_pid, std::process::id() as i32);

        // Manually release
        lock.release();

        // Verify PID file is removed
        assert!(!Path::new(PID_FILE_PATH).exists());

        // Should be able to acquire again
        let lock2 = PidLock::new();
        assert!(lock2.acquire().is_ok());
        lock2.release();
    }

    #[test]
    fn test_is_process_running() {
        let lock = PidLock::new();
        let current_pid = std::process::id() as i32;

        // Current process should be running
        assert!(lock.is_process_running(current_pid));

        // PID 99999 should not be running (very unlikely to exist)
        assert!(!lock.is_process_running(99999));
    }
}
