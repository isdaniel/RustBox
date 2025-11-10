use crate::ipc::{DaemonRequest, DaemonResponse, ContainerSummary};
use chrono::Utc;

#[test]
fn test_daemon_request_serialization() {
    let request = DaemonRequest::RunRequest {
        name: Some("test_container".to_string()),
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/echo".to_string(), "hello".to_string()],
        workdir: "/".to_string(),
        rootfs_path: "./rootfs".to_string(),
        tty: false,
        isolate_user: false,
        isolate_network: false,
    };
    
    // Serialize to JSON
    let json = serde_json::to_string(&request).expect("Failed to serialize request");
    assert!(!json.is_empty());
    
    // Deserialize from JSON
    let deserialized: DaemonRequest = serde_json::from_str(&json)
        .expect("Failed to deserialize request");
    
    // Verify it matches
    match (request, deserialized) {
        (DaemonRequest::RunRequest { name: n1, memory_limit: m1, .. }, 
         DaemonRequest::RunRequest { name: n2, memory_limit: m2, .. }) => {
            assert_eq!(n1, n2);
            assert_eq!(m1, m2);
        }
        _ => panic!("Request types don't match"),
    }
}

#[test]
fn test_daemon_response_serialization() {
    let response = DaemonResponse::RunResponse {
        container_id: "abc123def456".to_string(),
        name: "test_container".to_string(),
        state: "Running".to_string(),
    };
    
    // Test serialization round trip
    let json = serde_json::to_string(&response).unwrap();
    let deserialized: DaemonResponse = serde_json::from_str(&json).unwrap();
    
    match (response, deserialized) {
        (DaemonResponse::RunResponse { container_id: id1, name: n1, .. },
         DaemonResponse::RunResponse { container_id: id2, name: n2, .. }) => {
            assert_eq!(id1, id2);
            assert_eq!(n1, n2);
        }
        _ => panic!("Response types don't match"),
    }
}

#[test]
fn test_container_summary_serialization() {
    let summary = ContainerSummary {
        id: "container123".to_string(),
        name: "web_server".to_string(),
        state: "Running".to_string(),
        command: vec!["/bin/nginx".to_string()],
        created_at: Utc::now(),
        started_at: Some(Utc::now()),
        finished_at: None,
        uptime_seconds: Some(3600),
        exit_code: None,
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
    };
    
    let json = serde_json::to_string(&summary).unwrap();
    let deserialized: ContainerSummary = serde_json::from_str(&json).unwrap();
    
    assert_eq!(summary.id, deserialized.id);
    assert_eq!(summary.name, deserialized.name);
    assert_eq!(summary.state, deserialized.state);
    assert_eq!(summary.exit_code, deserialized.exit_code);
}

#[test]
fn test_stop_request_serialization() {
    let request = DaemonRequest::StopRequest {
        container_id: "test123".to_string(),
        timeout: 30,
    };
    
    let json = serde_json::to_string(&request).unwrap();
    let deserialized: DaemonRequest = serde_json::from_str(&json).unwrap();
    
    match (request, deserialized) {
        (DaemonRequest::StopRequest { container_id: id1, timeout: t1 },
         DaemonRequest::StopRequest { container_id: id2, timeout: t2 }) => {
            assert_eq!(id1, id2);
            assert_eq!(t1, t2);
        }
        _ => panic!("Request types don't match"),
    }
}

#[test]
fn test_list_request_serialization() {
    let request = DaemonRequest::ListRequest { all: true };
    
    let json = serde_json::to_string(&request).unwrap();
    let deserialized: DaemonRequest = serde_json::from_str(&json).unwrap();
    
    match (request, deserialized) {
        (DaemonRequest::ListRequest { all: a1 },
         DaemonRequest::ListRequest { all: a2 }) => {
            assert_eq!(a1, a2);
        }
        _ => panic!("Request types don't match"),
    }
}