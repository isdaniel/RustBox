use crate::constants::OVERLAY_BASE_DIR;
use crate::container::sandbox::umount_detach;
use serde::{Deserialize, Serialize};
use std::fs::{create_dir_all, remove_dir_all};
use std::io;
use std::path::PathBuf;

/// Runtime configuration for a container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    /// Memory limit (e.g., "256M", "1G", "512000")
    pub memory_limit: String,

    /// CPU limit as fraction of one core (e.g., "0.5" for 50%)
    pub cpu_limit: String,

    /// Command to execute inside container
    pub command: Vec<String>,

    /// Working directory inside container
    pub workdir: String,

    /// Base rootfs directory (read-only layer)
    pub rootfs_path: String,

    /// Allocate a pseudo-TTY for the container
    pub tty: bool,

    /// Isolate user namespace (CLONE_NEWUSER)
    pub isolate_user: bool,

    /// Isolate network namespace (CLONE_NEWNET)
    pub isolate_network: bool,
}

impl ContainerConfig {
    /// Validate configuration fields
    pub fn validate(&self) -> Result<(), String> {
        // Validate memory_limit format (e.g., "256M", "1G", "512000")
        self.validate_memory_limit()?;

        // Validate cpu_limit is parseable f64 > 0.0
        self.validate_cpu_limit()?;

        // Validate command is non-empty
        if self.command.is_empty() {
            return Err("Command cannot be empty".to_string());
        }

        // Validate workdir is absolute
        if !self.workdir.starts_with('/') {
            return Err(format!("Workdir must be absolute path: {}", self.workdir));
        }

        Ok(())
    }

    fn validate_memory_limit(&self) -> Result<(), String> {
        let limit = &self.memory_limit;

        // Check if it's just a number (bytes)
        if limit.parse::<u64>().is_ok() {
            return Ok(());
        }

        // Check if it has a unit suffix (M, G, K)
        let re = regex::Regex::new(r"^(\d+)([MGK])$").map_err(|e| e.to_string())?;
        if re.is_match(limit) {
            return Ok(());
        }

        Err(format!("Invalid memory limit format: {limit}"))
    }

    fn validate_cpu_limit(&self) -> Result<(), String> {
        let cpu: f64 = self
            .cpu_limit
            .parse()
            .map_err(|_| format!("Invalid CPU limit format: {}", self.cpu_limit))?;

        if cpu <= 0.0 {
            return Err("CPU limit must be positive".to_string());
        }

        Ok(())
    }
}

/// Paths for OverlayFS layers per container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayPaths {
    /// Lower directory (read-only base from rootfs)
    pub lower: PathBuf,

    /// Upper directory (read-write layer per container)
    pub upper: PathBuf,

    /// Work directory (overlay metadata)
    pub work: PathBuf,

    /// Merged directory (union mount point)
    pub merged: PathBuf,
}

impl OverlayPaths {
    /// Create paths for a new container
    pub fn new(container_id: &str, rootfs_base: &str) -> Self {
        let rootfs_path = PathBuf::from(rootfs_base);
        let rootfs_path = if rootfs_path.is_absolute() {
            rootfs_path
        } else {
            // Convert relative path to absolute by resolving it against current directory
            std::env::current_dir()
                .ok()
                .and_then(|cwd| cwd.join(&rootfs_path).canonicalize().ok())
                .unwrap_or(rootfs_path)
        };

        let overlay_container_dir = PathBuf::from(OVERLAY_BASE_DIR).join(container_id);

        Self {
            lower: rootfs_path.join("lowerdir"),
            upper: overlay_container_dir.join("upperdir"),
            work: overlay_container_dir.join("workdir"),
            merged: overlay_container_dir.join("merged"),
        }
    }

    /// Create all directories (except lower, which should exist)
    pub fn create_dirs(&self) -> io::Result<()> {
        create_dir_all(&self.upper)?;
        create_dir_all(&self.work)?;
        create_dir_all(&self.merged)?;
        Ok(())
    }

    /// Copy the source upperdir content to the container-specific upper directory
    ///
    /// This copies the contents from rootfs/upperdir to the container's isolated
    /// upper directory under OVERLAY_BASE_DIR, preventing pollution of the original
    /// repository upperdir.
    pub fn copy_upperdir_content(&self, rootfs_base: &str) -> io::Result<()> {
        let rootfs_path = PathBuf::from(rootfs_base);
        let rootfs_path = if rootfs_path.is_absolute() {
            rootfs_path
        } else {
            std::env::current_dir()
                .ok()
                .and_then(|cwd| cwd.join(&rootfs_path).canonicalize().ok())
                .unwrap_or(rootfs_path)
        };

        let source_upperdir = rootfs_path.join("upperdir");

        // Only copy if source upperdir exists
        if source_upperdir.exists() {
            Self::copy_dir_recursive(&source_upperdir, &self.upper)?;
        }

        Ok(())
    }

    /// Recursively copy directory contents
    fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) -> io::Result<()> {
        if !dst.exists() {
            create_dir_all(dst)?;
        }

        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());

            if src_path.is_dir() {
                Self::copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }

        Ok(())
    }

    /// Clean up all directories (merged, work, and upper)
    ///
    /// Note: lower directory is not removed as it's the shared read-only base layer
    pub fn cleanup(&self) -> io::Result<()> {
        if self.merged.exists() {
            umount_detach(&self.merged);
            remove_dir_all(&self.merged)?;
        }

        // Remove work directory
        if self.work.exists() {
            remove_dir_all(&self.work)?;
        }

        // Remove upper directory (container-specific writable layer)
        if self.upper.exists() {
            remove_dir_all(&self.upper)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_config_valid() {
        let config = ContainerConfig {
            memory_limit: "256M".to_string(),
            cpu_limit: "0.5".to_string(),
            command: vec!["/bin/bash".to_string()],
            workdir: "/".to_string(),
            rootfs_path: "./rootfs".to_string(),
            tty: false,
            isolate_user: false,
            isolate_network: false,
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_config_invalid_memory() {
        let config = ContainerConfig {
            memory_limit: "invalid".to_string(),
            cpu_limit: "0.5".to_string(),
            command: vec!["/bin/bash".to_string()],
            workdir: "/".to_string(),
            rootfs_path: "./rootfs".to_string(),
            tty: false,
            isolate_user: false,
            isolate_network: false,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_config_invalid_cpu() {
        let config = ContainerConfig {
            memory_limit: "256M".to_string(),
            cpu_limit: "invalid".to_string(),
            command: vec!["/bin/bash".to_string()],
            workdir: "/".to_string(),
            rootfs_path: "./rootfs".to_string(),
            tty: false,
            isolate_user: false,
            isolate_network: false,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_config_empty_command() {
        let config = ContainerConfig {
            memory_limit: "256M".to_string(),
            cpu_limit: "0.5".to_string(),
            command: vec![],
            workdir: "/".to_string(),
            rootfs_path: "./rootfs".to_string(),
            tty: false,
            isolate_user: false,
            isolate_network: false,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_config_relative_workdir() {
        let config = ContainerConfig {
            memory_limit: "256M".to_string(),
            cpu_limit: "0.5".to_string(),
            command: vec!["/bin/bash".to_string()],
            workdir: "relative/path".to_string(),
            rootfs_path: "./rootfs".to_string(),
            tty: false,
            isolate_user: false,
            isolate_network: false,
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_overlay_paths_new() {
        let paths = OverlayPaths::new("a3f7b2c4d5e6", "./rootfs");

        // The lower path gets canonicalized if it's a relative path
        let expected_lower = std::env::current_dir()
            .ok()
            .and_then(|cwd| cwd.join("./rootfs/lowerdir").canonicalize().ok())
            .unwrap_or_else(|| PathBuf::from("./rootfs/lowerdir"));

        assert_eq!(paths.lower, expected_lower);
        assert_eq!(
            paths.upper,
            PathBuf::from(OVERLAY_BASE_DIR).join("a3f7b2c4d5e6/upperdir")
        );
        assert_eq!(
            paths.work,
            PathBuf::from(OVERLAY_BASE_DIR).join("a3f7b2c4d5e6/workdir")
        );
        assert_eq!(
            paths.merged,
            PathBuf::from(OVERLAY_BASE_DIR).join("a3f7b2c4d5e6/merged")
        );
    }
}
