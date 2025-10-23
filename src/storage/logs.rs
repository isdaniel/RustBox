use std::fs;
use std::path::{Path, PathBuf};

use crate::constants::LOG_BASE_DIR;

/// Container log manager
pub struct ContainerLogs {
    container_id: String,
    log_dir: PathBuf,
}

impl ContainerLogs {
    /// Create a new container log manager
    pub fn new(container_id: String) -> Self {
        let log_dir = PathBuf::from(LOG_BASE_DIR).join(&container_id);
        Self {
            container_id,
            log_dir,
        }
    }

    /// Get the log directory path for this container
    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }

    /// Get the container ID
    pub fn container_id(&self) -> &str {
        &self.container_id
    }

    /// Get the stdout log file path
    pub fn stdout_path(&self) -> PathBuf {
        self.log_dir.join("stdout.log")
    }

    /// Get the stderr log file path
    pub fn stderr_path(&self) -> PathBuf {
        self.log_dir.join("stderr.log")
    }

    /// Create the log directory structure
    pub fn create_log_dir(&self) -> Result<(), String> {
        fs::create_dir_all(&self.log_dir).map_err(|e| {
            format!(
                "Failed to create log directory {}: {}",
                self.log_dir.display(),
                e
            )
        })?;
        Ok(())
    }

    /// Create stdout and stderr log files
    pub fn create_log_files(&self) -> Result<(), String> {
        // Create the directory first
        self.create_log_dir()?;

        // Create stdout.log
        fs::File::create(self.stdout_path())
            .map_err(|e| format!("Failed to create stdout log file: {e}"))?;

        // Create stderr.log
        fs::File::create(self.stderr_path())
            .map_err(|e| format!("Failed to create stderr log file: {e}"))?;

        Ok(())
    }

    /// Read stdout log contents
    pub fn read_stdout(&self) -> Result<String, String> {
        fs::read_to_string(self.stdout_path())
            .map_err(|e| format!("Failed to read stdout log: {e}"))
    }

    /// Read stderr log contents
    pub fn read_stderr(&self) -> Result<String, String> {
        fs::read_to_string(self.stderr_path())
            .map_err(|e| format!("Failed to read stderr log: {e}"))
    }

    /// Read last N lines from stdout log
    pub fn read_stdout_tail(&self, lines: usize) -> Result<Vec<String>, String> {
        self.read_tail(&self.stdout_path(), lines)
    }

    /// Read last N lines from stderr log
    pub fn read_stderr_tail(&self, lines: usize) -> Result<Vec<String>, String> {
        self.read_tail(&self.stderr_path(), lines)
    }

    /// Read last N lines from a log file efficiently
    fn read_tail(&self, path: &Path, lines: usize) -> Result<Vec<String>, String> {
        let contents = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read log file {}: {}", path.display(), e))?;

        let all_lines: Vec<String> = contents.lines().map(|s| s.to_string()).collect();
        let start_idx = all_lines.len().saturating_sub(lines);
        Ok(all_lines[start_idx..].to_vec())
    }

    /// Delete log files and directory for this container
    fn cleanup(&self) -> Result<(), String> {
        if self.log_dir.exists() {
            fs::remove_dir_all(&self.log_dir)
                .map_err(|e| format!("Failed to remove log directory: {e}"))?;
        }
        Ok(())
    }
}

impl Drop for ContainerLogs {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_paths() {
        let logs = ContainerLogs::new("test123".to_string());
        assert_eq!(logs.log_dir(), Path::new("/tmp/rustbox/logs/test123"));
        assert_eq!(
            logs.stdout_path(),
            PathBuf::from("/tmp/rustbox/logs/test123/stdout.log")
        );
        assert_eq!(
            logs.stderr_path(),
            PathBuf::from("/tmp/rustbox/logs/test123/stderr.log")
        );
    }
}
