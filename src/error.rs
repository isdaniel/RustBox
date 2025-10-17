use thiserror::Error;

/// Container-related errors
#[derive(Error, Debug)]
pub enum ContainerError {
    #[error("Container not found: {0}")]
    NotFound(String),

    #[error("Container already exists: {0}")]
    AlreadyExists(String),

    #[error("Container is in invalid state: expected {expected}, got {actual}")]
    InvalidState { expected: String, actual: String },

    #[error("Container failed to start: {0}")]
    StartFailed(String),

    #[error("Container failed to stop: {0}")]
    StopFailed(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Daemon-related errors
#[derive(Error, Debug)]
pub enum DaemonError {
    #[error("Failed to bind socket: {0}")]
    SocketBind(std::io::Error),

    #[error("Failed to accept connection: {0}")]
    SocketAccept(std::io::Error),

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Container error: {0}")]
    Container(#[from] ContainerError),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("IPC error: {0}")]
    Ipc(#[from] IpcError),

    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Daemon is already running with PID {0}. Stop it first or check if it's stale.")]
    DaemonAlreadyRunning(i32),
}

/// IPC protocol errors
#[derive(Error, Debug)]
pub enum IpcError {
    #[error("Failed to connect to daemon socket: {0}")]
    ConnectionFailed(std::io::Error),

    #[error("Failed to send message: {0}")]
    SendFailed(std::io::Error),

    #[error("Failed to receive message: {0}")]
    ReceiveFailed(std::io::Error),

    #[error("Invalid message format: {0}")]
    InvalidFormat(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Storage-related errors
#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Failed to save metadata: {0}")]
    SaveFailed(std::io::Error),

    #[error("Failed to load metadata: {0}")]
    LoadFailed(std::io::Error),

    #[error("Failed to delete metadata: {0}")]
    DeleteFailed(std::io::Error),

    #[error("Corrupted metadata file: {0}")]
    CorruptedMetadata(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Storage error: {0}")]
    Generic(String),
}

impl From<String> for StorageError {
    fn from(s: String) -> Self {
        StorageError::Generic(s)
    }
}

/// Registry-related errors
#[derive(Error, Debug)]
pub enum RegistryError {
    #[error("Duplicate container ID: {0}")]
    DuplicateId(String),

    #[error("Container not found in registry: {0}")]
    NotFound(String),
}

/// Sandbox execution errors (for backward compatibility with existing code)
#[derive(Error, Debug)]
pub enum SandboxError {
    #[error("Sandbox execution failed: {0}")]
    ExecutionFailed(String),

    #[error("Namespace setup failed: {0}")]
    NamespaceSetup(String),

    #[error("Cgroup setup failed: {0}")]
    CgroupSetup(String),

    #[error("Mount operation failed: {0}")]
    MountFailed(String),

    #[error("Fork operation failed: {0}")]
    ForkFailed(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
