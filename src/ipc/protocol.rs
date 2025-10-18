use crate::container::{Container, ContainerConfig};
use crate::error::IpcError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Request messages from client to daemon
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonRequest {
    RunRequest {
        name: Option<String>,
        memory_limit: String,
        cpu_limit: String,
        command: Vec<String>,
        workdir: String,
        rootfs_path: String,
        #[serde(default)]
        tty: bool,
    },
    StopRequest {
        container_id: String,
        #[serde(default = "default_timeout")]
        timeout: u64,
    },
    ListRequest {
        #[serde(default)]
        all: bool,
    },
    InspectRequest {
        container_id: String,
    },
    RemoveRequest {
        container_id: String,
        #[serde(default)]
        force: bool,
    },
    LogsRequest {
        container_id: String,
        #[serde(default = "default_tail_lines")]
        tail: usize,
    },
    AttachRequest {
        container_id: String,
    },
    StatusRequest,
    // Streaming attach messages
    AttachStdin {
        data: Vec<u8>,
    },
    AttachDetach,
}

fn default_timeout() -> u64 {
    10
}

fn default_tail_lines() -> usize {
    100
}

/// Response messages from daemon to client
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonResponse {
    RunResponse {
        container_id: String,
        name: String,
        state: String,
    },
    StopResponse {
        container_id: String,
        state: String,
    },
    ListResponse {
        containers: Vec<ContainerSummary>,
    },
    InspectResponse {
        container: Box<Container>,
    },
    RemoveResponse {
        container_id: String,
        message: String,
    },
    LogsResponse {
        container_id: String,
        stdout: Vec<String>,
        stderr: Vec<String>,
    },
    StatusResponse {
        pid: u32,
        uptime_seconds: u64,
        container_count: usize,
        running_count: usize,
    },
    AttachResponse {
        container_id: String,
        message: String,
    },
    // Streaming attach messages
    AttachStdout {
        data: Vec<u8>,
    },
    AttachStderr {
        data: Vec<u8>,
    },
    AttachDetach,
    AttachExit {
        exit_code: i32,
    },
    SuccessResponse {
        message: String,
    },
    ErrorResponse {
        message: String,
        code: u32,
    },
}

/// Container summary for list responses
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContainerSummary {
    pub id: String,
    pub name: String,
    pub state: String,
    pub command: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub uptime_seconds: Option<i64>,
    pub exit_code: Option<i32>,
    pub memory_limit: String,
    pub cpu_limit: String,
}

impl From<&Container> for ContainerSummary {
    fn from(container: &Container) -> Self {
        ContainerSummary {
            id: container.id.clone(),
            name: container.name.clone(),
            state: container.state.to_string(),
            command: container.config.command.clone(),
            created_at: container.created_at,
            started_at: container.started_at,
            finished_at: container.finished_at,
            uptime_seconds: container.uptime_seconds(),
            exit_code: container.exit_code,
            memory_limit: container.config.memory_limit.clone(),
            cpu_limit: container.config.cpu_limit.clone(),
        }
    }
}

impl DaemonRequest {
    /// Convert RunRequest fields into ContainerConfig
    pub fn to_container_config(&self) -> Option<ContainerConfig> {
        match self {
            DaemonRequest::RunRequest {
                memory_limit,
                cpu_limit,
                command,
                workdir,
                rootfs_path,
                tty,
                ..
            } => Some(ContainerConfig {
                memory_limit: memory_limit.clone(),
                cpu_limit: cpu_limit.clone(),
                command: command.clone(),
                workdir: workdir.clone(),
                rootfs_path: rootfs_path.clone(),
                tty: *tty,
            }),
            _ => None,
        }
    }
}

/// Generic helper to read a length-prefixed JSON message from a stream
///
/// Format: [4-byte length (u32, big-endian)][JSON payload]
pub async fn read_message<R, T>(reader: &mut R) -> Result<T, IpcError>
where
    R: AsyncReadExt + Unpin,
    T: serde::de::DeserializeOwned,
{
    // Read 4-byte length prefix (big-endian)
    let length = reader.read_u32().await?;

    if length > 1_000_000 {
        return Err(IpcError::InvalidFormat(format!(
            "Message too large: {length} bytes"
        )));
    }

    // Read JSON payload
    let mut buffer = vec![0u8; length as usize];
    reader.read_exact(&mut buffer).await?;

    // Deserialize JSON
    let message = serde_json::from_slice(&buffer)?;
    Ok(message)
}

/// Generic helper to write a length-prefixed JSON message to a stream
///
/// Format: [4-byte length (u32, big-endian)][JSON payload]
pub async fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), IpcError>
where
    W: AsyncWriteExt + Unpin,
    T: serde::Serialize,
{
    // Serialize to JSON
    let json = serde_json::to_vec(message)?;
    let length = json.len() as u32;

    // Write length prefix (big-endian)
    writer.write_u32(length).await?;

    // Write JSON payload
    writer.write_all(&json).await?;
    writer.flush().await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::BufReader;

    #[tokio::test]
    async fn test_message_framing() {
        let request = DaemonRequest::ListRequest { all: true };
        let json = serde_json::to_vec(&request).unwrap();
        let length = json.len() as u32;

        // Create a buffer with length prefix + JSON
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&length.to_be_bytes());
        buffer.extend_from_slice(&json);

        // Read message
        let mut reader = BufReader::new(&buffer[..]);
        let decoded = read_message(&mut reader).await.unwrap();

        match decoded {
            DaemonRequest::ListRequest { all } => assert!(all),
            _ => panic!("Unexpected request type"),
        }
    }

    #[tokio::test]
    async fn test_write_read_roundtrip() {
        let response = DaemonResponse::SuccessResponse {
            message: "Test".to_string(),
        };

        let mut buffer = Vec::new();
        write_message(&mut buffer, &response).await.unwrap();

        // Verify the buffer structure
        assert!(buffer.len() > 4);
        let length = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
        assert_eq!(buffer.len(), 4 + length as usize);
    }
}
