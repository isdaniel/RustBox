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
        let overlay_base = PathBuf::from("/var/lib/rustbox/overlay").join(container_id);
        Self {
            lower: PathBuf::from(rootfs_base).join("lowerdir"),
            upper: overlay_base.join("upper"),
            work: overlay_base.join("work"),
            merged: overlay_base.join("merged"),
        }
    }

    /// Create all directories (except lower, which should exist)
    pub fn create_dirs(&self) -> io::Result<()> {
        create_dir_all(&self.upper)?;
        create_dir_all(&self.work)?;
        create_dir_all(&self.merged)?;
        Ok(())
    }

    /// Clean up all directories (except lower, which is shared)
    pub fn cleanup(&self) -> io::Result<()> {
        // Remove upper, work, merged directories
        if self.upper.exists() {
            remove_dir_all(&self.upper)?;
        }
        if self.work.exists() {
            remove_dir_all(&self.work)?;
        }
        if self.merged.exists() {
            remove_dir_all(&self.merged)?;
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
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_overlay_paths_new() {
        let paths = OverlayPaths::new("a3f7b2c4d5e6", "./rootfs");
        assert_eq!(paths.lower, PathBuf::from("./rootfs/lowerdir"));
        assert_eq!(
            paths.upper,
            PathBuf::from("/var/lib/rustbox/overlay/a3f7b2c4d5e6/upper")
        );
        assert_eq!(
            paths.work,
            PathBuf::from("/var/lib/rustbox/overlay/a3f7b2c4d5e6/work")
        );
        assert_eq!(
            paths.merged,
            PathBuf::from("/var/lib/rustbox/overlay/a3f7b2c4d5e6/merged")
        );
    }
}
