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