use crate::container::{ContainerState, Container, ContainerConfig};

#[test]
fn test_container_state_transitions() {
    // Test valid transitions
    let mut state = ContainerState::Created;
    
    // Created -> Running
    assert!(state.can_start());
    state = ContainerState::Running;
    
    // Running -> Stopped
    assert!(state.can_stop());
    state = ContainerState::Stopped;
    
    // Stopped -> Running (restart) - this should be false based on the implementation
    assert!(!state.can_start());
    
    // Running -> Exited
    let state = ContainerState::Exited;
    assert!(state.is_terminal());
}

#[test]
fn test_invalid_state_transitions() {
    // Test invalid transitions
    let state = ContainerState::Exited;
    
    // Exited containers cannot be started
    assert!(!state.can_start());
    
    // Exited containers cannot be stopped
    assert!(!state.can_stop());
}

#[test]
fn test_container_state_queries() {
    // Test state query methods
    assert!(ContainerState::Running.is_running());
    assert!(!ContainerState::Created.is_running());
    assert!(!ContainerState::Stopped.is_running());
    assert!(!ContainerState::Exited.is_running());
    
    // Test removal eligibility
    assert!(ContainerState::Created.can_remove());
    assert!(!ContainerState::Running.can_remove());
    assert!(ContainerState::Stopped.can_remove());
    assert!(ContainerState::Exited.can_remove());
}

#[test]
fn test_container_lifecycle() {
    let config = ContainerConfig {
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/sh".to_string()],
        workdir: "/".to_string(),
        rootfs_path: "./rootfs".to_string(),
        tty: false,
    };
    
    let mut container = Container::new(Some("test".to_string()), config);
    
    // Initial state should be Created
    assert_eq!(container.state, ContainerState::Created);
    
    // Mark as started
    let pid = 1234;
    assert!(container.mark_started(pid).is_ok());
    assert_eq!(container.state, ContainerState::Running);
    assert_eq!(container.pid, Some(pid));
    
    // Mark as exited
    let exit_code = 0;
    assert!(container.mark_exited(exit_code).is_ok());
    assert_eq!(container.state, ContainerState::Exited);
    assert_eq!(container.exit_code, Some(exit_code));
}