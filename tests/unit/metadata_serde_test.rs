use crate::storage::ContainerMetadata;
use crate::container::{Container, ContainerConfig};

#[test]
fn test_container_metadata_serialization() {
    let config = ContainerConfig {
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/echo".to_string(), "hello".to_string()],
        workdir: "/".to_string(),
        rootfs_path: "./rootfs".to_string(),
        tty: false,
        isolate_user: false,
        isolate_network: false,
    };
    
    let container = Container::new(Some("test_container".to_string()), config);
    let metadata = ContainerMetadata::from(&container);
    
    // Serialize to JSON
    let json = serde_json::to_string(&metadata).expect("Failed to serialize metadata");
    assert!(!json.is_empty());
    
    // Deserialize from JSON
    let deserialized: ContainerMetadata = serde_json::from_str(&json)
        .expect("Failed to deserialize metadata");
    
    // Verify fields match
    assert_eq!(metadata.id, deserialized.id);
    assert_eq!(metadata.name, deserialized.name);
    assert_eq!(metadata.state, deserialized.state);
    assert_eq!(metadata.config.memory_limit, deserialized.config.memory_limit);
    assert_eq!(metadata.config.command, deserialized.config.command);
}

#[test]
fn test_container_metadata_round_trip() {
    let config = ContainerConfig {
        memory_limit: "1G".to_string(),
        cpu_limit: "2.0".to_string(),
        command: vec!["/bin/sh".to_string()],
        workdir: "/home".to_string(),
        rootfs_path: "/var/rootfs".to_string(),
        tty: false,
        isolate_user: false,
        isolate_network: false,
    };
    
    let mut container = Container::new(None, config);
    container.mark_started(12345).unwrap();
    
    let metadata = ContainerMetadata::from(&container);
    
    // Serialize to pretty JSON
    let json = serde_json::to_string_pretty(&metadata)
        .expect("Failed to serialize metadata");
    
    // Deserialize back
    let deserialized: ContainerMetadata = serde_json::from_str(&json)
        .expect("Failed to deserialize metadata");
    
    // Verify all fields preserved
    assert_eq!(metadata.id, deserialized.id);
    assert_eq!(metadata.name, deserialized.name);
    assert_eq!(metadata.pid, deserialized.pid);
    assert_eq!(metadata.exit_code, deserialized.exit_code);
    assert_eq!(metadata.created_at, deserialized.created_at);
    assert_eq!(metadata.started_at, deserialized.started_at);
}

#[test]
fn test_container_metadata_with_different_states() {
    let config = ContainerConfig {
        memory_limit: "256M".to_string(),
        cpu_limit: "0.5".to_string(),
        command: vec!["/bin/pwd".to_string()],
        workdir: "/tmp".to_string(),
        rootfs_path: "./test-rootfs".to_string(),
        tty: false,
        isolate_user: false,
        isolate_network: false,
    };
    
    // Test Created state
    let container_created = Container::new(Some("created_test".to_string()), config.clone());
    let metadata_created = ContainerMetadata::from(&container_created);
    let json_created = serde_json::to_string(&metadata_created).unwrap();
    assert!(json_created.contains("\"state\":\"Created\""));
    
    // Test Running state
    let mut container_running = Container::new(Some("running_test".to_string()), config.clone());
    container_running.mark_started(9999).unwrap();
    let metadata_running = ContainerMetadata::from(&container_running);
    let json_running = serde_json::to_string(&metadata_running).unwrap();
    assert!(json_running.contains("\"state\":\"Running\""));
    
    // Test Exited state
    let mut container_exited = Container::new(Some("exited_test".to_string()), config);
    container_exited.mark_started(8888).unwrap();
    container_exited.mark_exited(42).unwrap();
    let metadata_exited = ContainerMetadata::from(&container_exited);
    let json_exited = serde_json::to_string(&metadata_exited).unwrap();
    assert!(json_exited.contains("\"exit_code\":42"));
}

#[test]
fn test_container_metadata_stopped_state_persistence() {
    let config = ContainerConfig {
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/bash".to_string()],
        workdir: "/".to_string(),
        rootfs_path: "./rootfs".to_string(),
        tty: false,
        isolate_user: false,
        isolate_network: false,
    };
    
    // Create container, start it, then stop it
    let mut container = Container::new(Some("stopped_test".to_string()), config);
    container.mark_started(1234).unwrap();
    container.mark_stopped().unwrap();
    
    // Verify it's in Stopped state
    use crate::container::ContainerState;
    assert_eq!(container.state, ContainerState::Stopped);
    
    // Serialize to metadata
    let metadata = ContainerMetadata::from(&container);
    let json = serde_json::to_string(&metadata).unwrap();
    
    // Verify JSON contains Stopped state
    assert!(json.contains("\"state\":\"Stopped\""));
    
    // Deserialize back
    let deserialized: ContainerMetadata = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.state, ContainerState::Stopped);
    
    // Verify we can convert back to Container and it's still stopped
    let restored_container = Container::try_from(deserialized).unwrap();
    assert_eq!(restored_container.state, ContainerState::Stopped);
    assert!(restored_container.state.can_start());
}

#[test]
fn test_container_metadata_start_stop_restart_cycle() {
    let config = ContainerConfig {
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/sleep".to_string(), "1000".to_string()],
        workdir: "/".to_string(),
        rootfs_path: "./rootfs".to_string(),
        tty: false,
        isolate_user: false,
        isolate_network: false,
    };
    
    use crate::container::ContainerState;
    
    // Initial state
    let mut container = Container::new(Some("cycle_test".to_string()), config);
    assert_eq!(container.state, ContainerState::Created);
    
    // First start
    container.mark_started(1000).unwrap();
    let metadata1 = ContainerMetadata::from(&container);
    let json1 = serde_json::to_string(&metadata1).unwrap();
    let restored1: ContainerMetadata = serde_json::from_str(&json1).unwrap();
    assert_eq!(restored1.state, ContainerState::Running);
    
    // Stop
    container.mark_stopped().unwrap();
    let metadata2 = ContainerMetadata::from(&container);
    let json2 = serde_json::to_string(&metadata2).unwrap();
    let restored2: ContainerMetadata = serde_json::from_str(&json2).unwrap();
    assert_eq!(restored2.state, ContainerState::Stopped);
    
    // Restart (transition from Stopped to Running)
    container.mark_started(2000).unwrap();
    let metadata3 = ContainerMetadata::from(&container);
    let json3 = serde_json::to_string(&metadata3).unwrap();
    let restored3: ContainerMetadata = serde_json::from_str(&json3).unwrap();
    assert_eq!(restored3.state, ContainerState::Running);
    assert_eq!(restored3.pid, Some(2000));
    
    // Stop again
    container.mark_stopped().unwrap();
    let metadata4 = ContainerMetadata::from(&container);
    let json4 = serde_json::to_string(&metadata4).unwrap();
    let restored4: ContainerMetadata = serde_json::from_str(&json4).unwrap();
    assert_eq!(restored4.state, ContainerState::Stopped);
}

#[test]
fn test_exited_container_cannot_restart_after_persistence() {
    let config = ContainerConfig {
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/echo".to_string(), "done".to_string()],
        workdir: "/".to_string(),
        rootfs_path: "./rootfs".to_string(),
        tty: false,
        isolate_user: false,
        isolate_network: false,
    };
    
    use crate::container::ContainerState;
    
    // Create, start, and exit container
    let mut container = Container::new(Some("exited_test".to_string()), config);
    container.mark_started(5000).unwrap();
    container.mark_exited(0).unwrap();
    
    // Serialize and deserialize
    let metadata = ContainerMetadata::from(&container);
    let json = serde_json::to_string(&metadata).unwrap();
    let deserialized: ContainerMetadata = serde_json::from_str(&json).unwrap();
    
    // Restore container
    let mut restored_container = Container::try_from(deserialized).unwrap();
    assert_eq!(restored_container.state, ContainerState::Exited);
    
    // Verify it cannot be restarted
    assert!(!restored_container.state.can_start());
    assert!(restored_container.mark_started(6000).is_err());
}

#[test]
fn test_container_metadata_preserves_timestamps_across_state_changes() {
    let config = ContainerConfig {
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/test".to_string()],
        workdir: "/".to_string(),
        rootfs_path: "./rootfs".to_string(),
        tty: false,
        isolate_user: false,
        isolate_network: false,
    };
    
    let mut container = Container::new(Some("timestamp_test".to_string()), config);
    let created_at = container.created_at;
    
    // Start container
    container.mark_started(3000).unwrap();
    let started_at = container.started_at.unwrap();
    
    // Serialize and deserialize
    let metadata1 = ContainerMetadata::from(&container);
    let json1 = serde_json::to_string(&metadata1).unwrap();
    let restored1: ContainerMetadata = serde_json::from_str(&json1).unwrap();
    
    assert_eq!(restored1.created_at, created_at);
    assert_eq!(restored1.started_at.unwrap(), started_at);
    
    // Stop container
    container.mark_stopped().unwrap();
    
    // Serialize and deserialize again
    let metadata2 = ContainerMetadata::from(&container);
    let json2 = serde_json::to_string(&metadata2).unwrap();
    let restored2: ContainerMetadata = serde_json::from_str(&json2).unwrap();
    
    // Timestamps should be preserved
    assert_eq!(restored2.created_at, created_at);
    assert_eq!(restored2.started_at.unwrap(), started_at);
}