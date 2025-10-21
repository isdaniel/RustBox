//! Centralized constants for RustBox
//!
//! This module contains all configuration constants used throughout the application,
//! especially path-related constants.

// ============================================================================
// Daemon Configuration
// ============================================================================

/// Unix socket path for daemon-client communication
pub const SOCKET_PATH: &str = "/tmp/rustbox-daemon.sock";

/// PID file path for daemon process locking
pub const PID_FILE_PATH: &str = "/tmp/rustbox-daemon.pid";

// ============================================================================
// Storage Paths
// ============================================================================

/// Base directory for container metadata storage
pub const METADATA_DIR: &str = "/tmp/rustbox/containers";

/// Base directory for container logs
pub const LOG_BASE_DIR: &str = "/tmp/rustbox/logs";

/// Base directory for overlay filesystem layers
pub const OVERLAY_BASE_DIR: &str = "/tmp/rustbox/overlay";

// ============================================================================
// Cgroup Paths
// ============================================================================

/// Base path for cgroup filesystem
pub const CGROUP_BASE: &str = "/sys/fs/cgroup";

/// Cgroup namespace for RustBox containers
pub const CGROUP_NAMESPACE: &str = "rustbox";

// ============================================================================
// Container Name Generation
// ============================================================================

/// Adjectives used for generating random container names
pub const NAME_ADJECTIVES: &[&str] = &[
    "happy", "eager", "brave", "calm", "wise", "bold", "swift", "clever", "kind", "noble",
    "gentle", "bright", "fierce", "proud", "jolly",
];

/// Nouns used for generating random container names
pub const NAME_NOUNS: &[&str] = &[
    "ferris",
    "crab",
    "lobster",
    "shrimp",
    "whale",
    "dolphin",
    "octopus",
    "seal",
    "otter",
    "penguin",
    "turtle",
    "starfish",
    "jellyfish",
    "shark",
    "ray",
];

pub const ESCAPE_BYTE: u8 = 0x1B;