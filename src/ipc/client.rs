use crate::constants::SOCKET_PATH;
use crate::error::IpcError;
use crate::ipc::{read_message, write_message, DaemonRequest, DaemonResponse};
use tokio::net::UnixStream;

/// IPC client for communicating with the RustBox daemon
pub struct IpcClient {
    stream: UnixStream,
}

impl IpcClient {
    /// Connect to the daemon
    pub async fn connect() -> Result<Self, IpcError> {
        let stream = UnixStream::connect(SOCKET_PATH)
            .await
            .map_err(IpcError::ConnectionFailed)?;

        Ok(Self { stream })
    }

    /// Send a request and receive a response
    pub async fn send_request(
        &mut self,
        request: DaemonRequest,
    ) -> Result<DaemonResponse, IpcError> {
        write_message(&mut self.stream, &request).await?;
        read_message(&mut self.stream).await
    }
}
