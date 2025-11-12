use serde::{Deserialize, Serialize};

/// Container lifecycle states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerState {
    /// Container metadata created, not yet started
    Created,

    /// Container process is running
    Running,

    /// Container was stopped by user (SIGTERM/SIGKILL)
    Stopped,

    /// Container process exited (with exit code)
    Exited,
}

impl ContainerState {
    /// Check if a state transition is valid
    ///
    /// Valid transitions:
    /// - Created → Running
    /// - Running → Stopped
    /// - Running → Exited
    /// - Stopped → Running (restart)
    /// - Stopped → Exited
    ///
    /// Invalid: Cannot transition from Exited
    pub fn can_transition_to(&self, new_state: ContainerState) -> bool {
        use ContainerState::*;
        matches!(
            (self, new_state),
            (Created, Running) | (Running, Stopped) | (Running, Exited) | (Stopped, Running) | (Stopped, Exited)
        )
    }

    /// Perform a state transition, returning error if invalid
    pub fn transition(&mut self, new_state: ContainerState) -> Result<(), String> {
        if self.can_transition_to(new_state) {
            *self = new_state;
            Ok(())
        } else {
            Err(format!(
                "Invalid state transition: {self:?} -> {new_state:?}"
            ))
        }
    }

    /// Check if container is in a terminal state
    pub fn is_terminal(&self) -> bool {
        matches!(self, ContainerState::Exited)
    }

    /// Check if container is running
    pub fn is_running(&self) -> bool {
        matches!(self, ContainerState::Running)
    }

    /// Check if container can be started
    pub fn can_start(&self) -> bool {
        matches!(self, ContainerState::Created | ContainerState::Stopped)
    }

    /// Check if container can be stopped
    pub fn can_stop(&self) -> bool {
        matches!(self, ContainerState::Running)
    }

    /// Check if container can be removed
    pub fn can_remove(&self) -> bool {
        !matches!(self, ContainerState::Running)
    }
}

impl std::fmt::Display for ContainerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerState::Created => write!(f, "Created"),
            ContainerState::Running => write!(f, "Running"),
            ContainerState::Stopped => write!(f, "Stopped"),
            ContainerState::Exited => write!(f, "Exited"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_transitions() {
        let mut state = ContainerState::Created;
        assert!(state.transition(ContainerState::Running).is_ok());
        assert_eq!(state, ContainerState::Running);

        assert!(state.transition(ContainerState::Stopped).is_ok());
        assert_eq!(state, ContainerState::Stopped);

        assert!(state.transition(ContainerState::Exited).is_ok());
        assert_eq!(state, ContainerState::Exited);
    }

    #[test]
    fn test_invalid_transitions() {
        let mut state = ContainerState::Exited;
        assert!(state.transition(ContainerState::Running).is_err());
        assert!(state.transition(ContainerState::Stopped).is_err());
        assert!(state.transition(ContainerState::Created).is_err());

        let mut state = ContainerState::Stopped;
        assert!(state.transition(ContainerState::Created).is_err());
    }

    #[test]
    fn test_running_to_exited() {
        let mut state = ContainerState::Running;
        assert!(state.transition(ContainerState::Exited).is_ok());
        assert_eq!(state, ContainerState::Exited);
    }

    #[test]
    fn test_is_terminal() {
        assert!(!ContainerState::Created.is_terminal());
        assert!(!ContainerState::Running.is_terminal());
        assert!(!ContainerState::Stopped.is_terminal());
        assert!(ContainerState::Exited.is_terminal());
    }

    #[test]
    fn test_is_running() {
        assert!(!ContainerState::Created.is_running());
        assert!(ContainerState::Running.is_running());
        assert!(!ContainerState::Stopped.is_running());
        assert!(!ContainerState::Exited.is_running());
    }

    #[test]
    fn test_can_operations() {
        assert!(ContainerState::Created.can_start());
        assert!(ContainerState::Stopped.can_start());
        assert!(!ContainerState::Running.can_start());
        assert!(!ContainerState::Exited.can_start());

        assert!(ContainerState::Running.can_stop());
        assert!(!ContainerState::Created.can_stop());
        assert!(!ContainerState::Stopped.can_stop());
        assert!(!ContainerState::Exited.can_stop());

        assert!(ContainerState::Created.can_remove());
        assert!(!ContainerState::Running.can_remove());
        assert!(ContainerState::Stopped.can_remove());
        assert!(ContainerState::Exited.can_remove());
    }

    #[test]
    fn test_stopped_to_running_transition() {
        let mut state = ContainerState::Stopped;
        assert!(state.transition(ContainerState::Running).is_ok());
        assert_eq!(state, ContainerState::Running);
    }

    #[test]
    fn test_exited_cannot_transition_to_running() {
        let mut state = ContainerState::Exited;
        assert!(state.transition(ContainerState::Running).is_err());
        assert_eq!(state, ContainerState::Exited);
    }

    #[test]
    fn test_multiple_start_stop_cycles() {
        // Test that we can cycle between Running and Stopped multiple times
        let mut state = ContainerState::Created;
        
        // First cycle: Created -> Running -> Stopped
        assert!(state.transition(ContainerState::Running).is_ok());
        assert_eq!(state, ContainerState::Running);
        assert!(state.transition(ContainerState::Stopped).is_ok());
        assert_eq!(state, ContainerState::Stopped);
        
        // Second cycle: Stopped -> Running -> Stopped
        assert!(state.transition(ContainerState::Running).is_ok());
        assert_eq!(state, ContainerState::Running);
        assert!(state.transition(ContainerState::Stopped).is_ok());
        assert_eq!(state, ContainerState::Stopped);
        
        // Third cycle: Stopped -> Running -> Stopped
        assert!(state.transition(ContainerState::Running).is_ok());
        assert_eq!(state, ContainerState::Running);
        assert!(state.transition(ContainerState::Stopped).is_ok());
        assert_eq!(state, ContainerState::Stopped);
    }

    #[test]
    fn test_stopped_cannot_transition_to_created() {
        let mut state = ContainerState::Stopped;
        assert!(state.transition(ContainerState::Created).is_err());
        assert_eq!(state, ContainerState::Stopped);
    }

    #[test]
    fn test_stopped_can_transition_to_exited() {
        let mut state = ContainerState::Stopped;
        assert!(state.transition(ContainerState::Exited).is_ok());
        assert_eq!(state, ContainerState::Exited);
    }

    #[test]
    fn test_exited_is_final_state() {
        let mut state = ContainerState::Exited;
        
        // Cannot transition from Exited to any other state
        assert!(state.transition(ContainerState::Created).is_err());
        assert!(state.transition(ContainerState::Running).is_err());
        assert!(state.transition(ContainerState::Stopped).is_err());
        
        // State should remain Exited
        assert_eq!(state, ContainerState::Exited);
    }

    #[test]
    fn test_running_cannot_transition_to_created() {
        let mut state = ContainerState::Running;
        assert!(state.transition(ContainerState::Created).is_err());
        assert_eq!(state, ContainerState::Running);
    }

    #[test]
    fn test_created_cannot_skip_to_stopped() {
        let mut state = ContainerState::Created;
        assert!(state.transition(ContainerState::Stopped).is_err());
        assert_eq!(state, ContainerState::Created);
    }

    #[test]
    fn test_created_cannot_go_to_exited() {
        let mut state = ContainerState::Created;
        assert!(state.transition(ContainerState::Exited).is_err());
        assert_eq!(state, ContainerState::Created);
    }

    #[test]
    fn test_all_states_display() {
        assert_eq!(ContainerState::Created.to_string(), "Created");
        assert_eq!(ContainerState::Running.to_string(), "Running");
        assert_eq!(ContainerState::Stopped.to_string(), "Stopped");
        assert_eq!(ContainerState::Exited.to_string(), "Exited");
    }

    #[test]
    fn test_state_query_combinations() {
        // Created state
        let created = ContainerState::Created;
        assert!(!created.is_running());
        assert!(!created.is_terminal());
        assert!(created.can_start());
        assert!(!created.can_stop());
        assert!(created.can_remove());

        // Running state
        let running = ContainerState::Running;
        assert!(running.is_running());
        assert!(!running.is_terminal());
        assert!(!running.can_start());
        assert!(running.can_stop());
        assert!(!running.can_remove());

        // Stopped state
        let stopped = ContainerState::Stopped;
        assert!(!stopped.is_running());
        assert!(!stopped.is_terminal());
        assert!(stopped.can_start());
        assert!(!stopped.can_stop());
        assert!(stopped.can_remove());

        // Exited state
        let exited = ContainerState::Exited;
        assert!(!exited.is_running());
        assert!(exited.is_terminal());
        assert!(!exited.can_start());
        assert!(!exited.can_stop());
        assert!(exited.can_remove());
    }

    #[test]
    fn test_transition_error_messages() {
        let mut state = ContainerState::Exited;
        
        let result = state.transition(ContainerState::Running);
        assert!(result.is_err());
        
        let err_msg = result.unwrap_err();
        assert!(err_msg.contains("Invalid state transition"));
        assert!(err_msg.contains("Exited"));
        assert!(err_msg.contains("Running"));
    }
}
