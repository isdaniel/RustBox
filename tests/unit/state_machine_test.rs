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
    
    // Stopped -> Running (restart) - this is now allowed for restart functionality
    assert!(state.can_start());
    
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
        isolate_user: false,
        isolate_network: false,
        env: vec![],
        pids_limit: None,
        cpu_weight: None,
        memory_swap_limit: None,
        port_mappings: vec![],
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

#[test]
fn test_container_start_stop_start_lifecycle() {
    let config = ContainerConfig {
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/sh".to_string()],
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
    };
    
    let mut container = Container::new(Some("restart_test".to_string()), config);
    
    // Initial state
    assert_eq!(container.state, ContainerState::Created);
    assert!(container.state.can_start());
    
    // First start
    assert!(container.mark_started(1001).is_ok());
    assert_eq!(container.state, ContainerState::Running);
    assert_eq!(container.pid, Some(1001));
    assert!(!container.state.can_start());
    assert!(container.state.can_stop());
    
    // Stop
    assert!(container.mark_stopped().is_ok());
    assert_eq!(container.state, ContainerState::Stopped);
    assert!(container.state.can_start());
    assert!(!container.state.can_stop());
    
    // Restart (this is the key test - Stopped -> Running)
    assert!(container.mark_started(1002).is_ok());
    assert_eq!(container.state, ContainerState::Running);
    assert_eq!(container.pid, Some(1002));
    assert!(!container.state.can_start());
    assert!(container.state.can_stop());
    
    // Stop again
    assert!(container.mark_stopped().is_ok());
    assert_eq!(container.state, ContainerState::Stopped);
    assert!(container.state.can_start());
}

#[test]
fn test_container_cannot_restart_after_exit() {
    let config = ContainerConfig {
        memory_limit: "256M".to_string(),
        cpu_limit: "0.5".to_string(),
        command: vec!["/bin/false".to_string()],
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
    };
    
    let mut container = Container::new(Some("exit_test".to_string()), config);
    
    // Start and exit
    assert!(container.mark_started(2001).is_ok());
    assert!(container.mark_exited(1).is_ok());
    assert_eq!(container.state, ContainerState::Exited);
    assert_eq!(container.exit_code, Some(1));
    
    // Cannot restart exited container
    assert!(!container.state.can_start());
    assert!(container.mark_started(2002).is_err());
    assert_eq!(container.state, ContainerState::Exited);
}

#[test]
fn test_container_multiple_restart_cycles() {
    let config = ContainerConfig {
        memory_limit: "1G".to_string(),
        cpu_limit: "2.0".to_string(),
        command: vec!["/bin/nginx".to_string()],
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
    };
    
    let mut container = Container::new(Some("multi_restart".to_string()), config);
    
    // Cycle 1
    assert!(container.mark_started(3001).is_ok());
    assert!(container.mark_stopped().is_ok());
    
    // Cycle 2
    assert!(container.mark_started(3002).is_ok());
    assert!(container.mark_stopped().is_ok());
    
    // Cycle 3
    assert!(container.mark_started(3003).is_ok());
    assert!(container.mark_stopped().is_ok());
    
    // Cycle 4
    assert!(container.mark_started(3004).is_ok());
    assert_eq!(container.state, ContainerState::Running);
    assert_eq!(container.pid, Some(3004));
}

#[test]
fn test_container_stopped_to_exited_transition() {
    let config = ContainerConfig {
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/sleep".to_string()],
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
    };
    
    let mut container = Container::new(Some("stop_exit_test".to_string()), config);
    
    // Start and stop
    assert!(container.mark_started(4001).is_ok());
    assert!(container.mark_stopped().is_ok());
    assert_eq!(container.state, ContainerState::Stopped);
    
    // Transition from Stopped to Exited (e.g., cleanup scenario)
    assert!(container.mark_exited(0).is_ok());
    assert_eq!(container.state, ContainerState::Exited);
    assert_eq!(container.exit_code, Some(0));
    
    // Cannot start again after exiting
    assert!(!container.state.can_start());
}

#[test]
fn test_container_state_cannot_go_backwards() {
    let config = ContainerConfig {
        memory_limit: "512M".to_string(),
        cpu_limit: "1.0".to_string(),
        command: vec!["/bin/true".to_string()],
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
    };
    
    let mut container = Container::new(Some("backwards_test".to_string()), config);
    
    // Running cannot go back to Created
    assert!(container.mark_started(5001).is_ok());
    assert_eq!(container.state, ContainerState::Running);
    assert!(container.state.transition(ContainerState::Created).is_err());
    
    // Stopped cannot go back to Created
    assert!(container.mark_stopped().is_ok());
    assert_eq!(container.state, ContainerState::Stopped);
    assert!(container.state.transition(ContainerState::Created).is_err());
    
    // Exited cannot go back to any state
    assert!(container.mark_exited(0).is_ok());
    assert_eq!(container.state, ContainerState::Exited);
    assert!(container.state.transition(ContainerState::Created).is_err());
    assert!(container.state.transition(ContainerState::Running).is_err());
    assert!(container.state.transition(ContainerState::Stopped).is_err());
}