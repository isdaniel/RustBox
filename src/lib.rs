//! # RustBox
//!
//! A Docker-like container runtime written in Rust with daemon architecture.
//!
//! RustBox provides multi-container orchestration, persistent state management,
//! and comprehensive CLI commands for container lifecycle management.
//!
//! ## Architecture
//!
//! RustBox consists of two main components:
//! - **Daemon** (`rustboxd`): Background service that manages containers
//! - **Client** (`rustbox`): CLI tool that communicates with the daemon
//!
//! ## Key Features
//!
//! - Multi-container management with persistent state
//! - Complete process isolation using Linux namespaces
//! - Resource limits via cgroups v2 (memory, CPU)
//! - Filesystem isolation using overlayfs
//! - Docker-like CLI interface
//! - Real-time logging and container attachment
//! - Graceful shutdown and state recovery
//!
//! ## Usage
//!
//! Start the daemon:
//! ```bash
//! sudo rustboxd
//! ```
//!
//! Create and manage containers:
//! ```bash
//! rustbox run --name myapp --memory 256M /bin/bash
//! rustbox list
//! rustbox logs myapp
//! rustbox stop myapp
//! rustbox remove myapp
//! ```
//!
//! ## Example
//!
//! ```rust,no_run
//! use rustbox::{run_sandbox, SandboxConfig};
//!
//! let config = SandboxConfig {
//!     memory_limit: "256M".to_string(),
//!     cpu_limit: "0.5".to_string(),
//!     command: vec!["/bin/bash".to_string()],
//!     workdir: "/".to_string(),
//!     stdout_log_path: None,
//!     stderr_log_path: None,
//!     tty: false,
//!     isolate_user: false,
//!     isolate_network: false,
//!     lower_dir: todo!(),
//!     upper_dir: todo!(),
//!     work_dir: todo!(),
//!     merged_dir: todo!(),
//! };
//!
//! // Run a sandboxed container (requires root privileges)
//! let result = run_sandbox(config).expect("Failed to run sandbox");
//! // The result contains PTY master FD (if TTY enabled), child PID, and cleanup paths
//! ```

// Re-export public modules
pub mod cli;
pub mod constants;
pub mod container;
pub mod error;
pub mod ipc;
pub mod storage;

// Re-export commonly used types for backward compatibility
pub use container::{run_sandbox, SandboxConfig};

#[cfg(test)]
mod tests {
    mod unit {
        mod state_machine_test {
            include!("../tests/unit/state_machine_test.rs");
        }

        mod metadata_serde_test {
            include!("../tests/unit/metadata_serde_test.rs");
        }

        mod ipc_protocol_test {
            include!("../tests/unit/ipc_protocol_test.rs");
        }
    }
}
