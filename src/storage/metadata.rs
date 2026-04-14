use crate::constants::METADATA_DIR;
use crate::container::{Container, ContainerConfig, ContainerState, OverlayPaths};
use crate::container::network::NetworkConfig;
use crate::error::StorageError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Serializable container metadata for disk persistence
#[derive(Debug, Serialize, Deserialize)]
pub struct ContainerMetadata {
    pub id: String,
    pub name: String,
    pub state: ContainerState,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub config: ContainerConfig,
    pub overlay_paths: OverlayPaths,
    pub cgroup_path: PathBuf,
    pub pid: Option<i32>,
    pub has_tty: bool,
    #[serde(default)]
    pub network_config: Option<NetworkConfig>,
}

impl From<&Container> for ContainerMetadata {
    fn from(container: &Container) -> Self {
        Self {
            id: container.id.clone(),
            name: container.name.clone(),
            state: container.state,
            created_at: container.created_at,
            started_at: container.started_at,
            finished_at: container.finished_at,
            exit_code: container.exit_code,
            config: container.config.clone(),
            overlay_paths: container.overlay_paths.clone(),
            cgroup_path: container.cgroup_path.clone(),
            pid: container.pid,
            has_tty: container.pty_master.is_some(),
            network_config: container.network_config.clone(),
        }
    }
}

impl From<ContainerMetadata> for Container {
    fn from(metadata: ContainerMetadata) -> Self {
        Container {
            id: metadata.id,
            name: metadata.name,
            state: metadata.state,
            created_at: metadata.created_at,
            started_at: metadata.started_at,
            finished_at: metadata.finished_at,
            exit_code: metadata.exit_code,
            config: metadata.config,
            overlay_paths: metadata.overlay_paths,
            cgroup_path: metadata.cgroup_path,
            pid: metadata.pid,
            pty_master: None, // PTY master FD is not persisted, will be recreated if needed
            network_config: metadata.network_config,
        }
    }
}

/// Save container metadata to disk
pub fn save_metadata(container: &Container) -> Result<(), StorageError> {
    let metadata = ContainerMetadata::from(container);
    let metadata_dir = Path::new(METADATA_DIR);

    // Create directory if it doesn't exist
    fs::create_dir_all(metadata_dir).map_err(StorageError::SaveFailed)?;

    let metadata_path = metadata_dir.join(format!("{}.json", container.id));
    let json = serde_json::to_string_pretty(&metadata)?;

    // Write to temp file then rename for atomicity
    let temp_path = metadata_path.with_extension("json.tmp");
    let mut file = File::create(&temp_path).map_err(StorageError::SaveFailed)?;
    file.write_all(json.as_bytes())
        .map_err(StorageError::SaveFailed)?;
    file.sync_all().map_err(StorageError::SaveFailed)?;

    fs::rename(&temp_path, &metadata_path).map_err(StorageError::SaveFailed)?;

    Ok(())
}

/// Load container metadata from disk
pub fn load_metadata(container_id: &str) -> Result<Container, StorageError> {
    let metadata_path = Path::new(METADATA_DIR).join(format!("{container_id}.json"));
    let json = fs::read_to_string(&metadata_path).map_err(StorageError::LoadFailed)?;

    let metadata: ContainerMetadata =
        serde_json::from_str(&json).map_err(|e| StorageError::CorruptedMetadata(e.to_string()))?;

    Ok(metadata.into())
}

/// Load all container metadata from disk
pub fn load_all_metadata() -> Result<Vec<Container>, StorageError> {
    let metadata_dir = Path::new(METADATA_DIR);

    // Create directory if it doesn't exist
    if !metadata_dir.exists() {
        fs::create_dir_all(metadata_dir).map_err(StorageError::LoadFailed)?;
        return Ok(Vec::new());
    }

    let mut containers = Vec::new();

    for entry in fs::read_dir(metadata_dir).map_err(StorageError::LoadFailed)? {
        let entry = entry.map_err(StorageError::LoadFailed)?;
        let path = entry.path();

        // Skip non-JSON files
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        match fs::read_to_string(&path) {
            Ok(json) => {
                match serde_json::from_str::<ContainerMetadata>(&json) {
                    Ok(metadata) => {
                        containers.push(metadata.into());
                    }
                    Err(e) => {
                        // Handle corrupted metadata: backup and skip
                        let backup_path = path.with_extension("json.corrupted");
                        let _ = fs::rename(&path, &backup_path);
                        tracing::warn!(
                            "Corrupted metadata file {}: {}. Backed up to {}",
                            path.display(),
                            e,
                            backup_path.display()
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read metadata file {}: {}", path.display(), e);
            }
        }
    }

    Ok(containers)
}

/// Delete container metadata from disk
pub fn delete_metadata(container_id: &str) -> Result<(), StorageError> {
    let metadata_path = Path::new(METADATA_DIR).join(format!("{container_id}.json"));

    if metadata_path.exists() {
        fs::remove_file(&metadata_path).map_err(StorageError::DeleteFailed)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::ContainerConfig;

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
            env: vec![],
            pids_limit: None,
            cpu_weight: None,
            memory_swap_limit: None,
            port_mappings: vec![],
        }
    }

    #[test]
    fn test_metadata_conversion() {
        let container = Container::new(Some("test".to_string()), test_config());
        let metadata = ContainerMetadata::from(&container);

        assert_eq!(metadata.id, container.id);
        assert_eq!(metadata.name, container.name);
        assert_eq!(metadata.state, container.state);

        let container2: Container = metadata.into();
        assert_eq!(container2.id, container.id);
        assert_eq!(container2.name, container.name);
    }
}
