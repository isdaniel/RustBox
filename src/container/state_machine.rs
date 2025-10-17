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
    /// - Stopped → Exited
    ///
    /// Invalid: Cannot transition from Exited or Stopped back to Running
    pub fn can_transition_to(&self, new_state: ContainerState) -> bool {
        use ContainerState::*;
        matches!(
            (self, new_state),
            (Created, Running) | (Running, Stopped) | (Running, Exited) | (Stopped, Exited)
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
        matches!(self, ContainerState::Created)
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

        let mut state = ContainerState::Stopped;
        assert!(state.transition(ContainerState::Running).is_err());
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
        assert!(!ContainerState::Running.can_start());

        assert!(ContainerState::Running.can_stop());
        assert!(!ContainerState::Created.can_stop());

        assert!(ContainerState::Created.can_remove());
        assert!(!ContainerState::Running.can_remove());
        assert!(ContainerState::Stopped.can_remove());
        assert!(ContainerState::Exited.can_remove());
    }
}
