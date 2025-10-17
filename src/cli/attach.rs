use crate::constants::SOCKET_PATH;
use crate::error::IpcError;
use crate::ipc::{read_response, write_request, DaemonRequest, DaemonResponse};
use clap::Args;
use std::io::{self, Write};
use termion::event::Key;
use termion::input::TermRead;
use termion::raw::IntoRawMode;
use tokio::net::UnixStream;

#[derive(Args, Debug)]
pub struct AttachArgs {
    pub container_id: String,
}

pub async fn execute(args: AttachArgs) -> Result<(), IpcError> {
    tracing::info!(container_id = %args.container_id, "Initiating attach to container");
    tracing::info!("Attaching to container: {}", args.container_id);

    // Connect to daemon
    tracing::debug!("Connecting to daemon socket");
    let mut stream = UnixStream::connect(SOCKET_PATH)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to connect to daemon");
            IpcError::ConnectionFailed(e)
        })?;

    tracing::debug!("Connected to daemon, sending attach request");
    // Send initial attach request
    let request = DaemonRequest::AttachRequest {
        container_id: args.container_id.clone(),
    };

    write_request(&mut stream, &request).await?;
    let response = read_response(&mut stream).await?;

    match response {
        DaemonResponse::AttachResponse {
            container_id,
            message,
        } => {
            tracing::info!("Attach request accepted");
            tracing::info!("Attached to container: {container_id}");
            tracing::info!("{message}");

            // Start real streaming attach session
            tracing::debug!("Starting streaming attach session");
            streaming_attach_session(&args.container_id, stream).await?;

            tracing::info!("Attach session ended");
            Ok(())
        }
        DaemonResponse::ErrorResponse { message, .. } => {
            tracing::error!(error = %message, "Daemon returned error");
            tracing::error!("Error: {message}");
            Err(IpcError::InvalidFormat(message))
        }
        _ => {
            tracing::error!("Unexpected response type from daemon");
            tracing::error!("Unexpected response type");
            Err(IpcError::InvalidFormat("Unexpected response".to_string()))
        }
    }
}

/// Handle real streaming attach session with PTY forwarding
async fn streaming_attach_session(container_id: &str, stream: UnixStream) -> Result<(), IpcError> {
    tracing::info!(container_id = %container_id, "Starting streaming attach session");
    tracing::info!("Starting streaming attach to container: {container_id}");
    tracing::info!("Press Ctrl+P followed by Ctrl+Q to detach, or Ctrl+C to exit");

    // Setup raw mode for terminal
    tracing::debug!("Entering raw terminal mode");
    let _stdout = io::stdout().into_raw_mode().map_err(|e| {
        tracing::error!(error = %e, "Failed to enter raw mode");
        IpcError::InvalidFormat(format!("Failed to enter raw mode: {e}"))
    })?;

    // Split stream for bidirectional communication
    let (mut stream_read, mut stream_write) = stream.into_split();

    // Spawn task to handle daemon responses (container output)
    tracing::debug!("Spawning output forwarding task");
    let output_task = tokio::spawn(async move {
        loop {
            match read_response(&mut stream_read).await {
                Ok(DaemonResponse::AttachStdout { data }) => {
                    // Write container output to our stdout
                    if let Err(e) = io::stdout().write_all(&data) {
                        tracing::error!("\r\nError writing output: {e}\r");
                        break;
                    }
                    let _ = io::stdout().flush();
                }
                Ok(DaemonResponse::AttachDetach) => {
                    tracing::info!("\r\nContainer detached\r");
                    break;
                }
                Ok(DaemonResponse::ErrorResponse { message, .. }) => {
                    tracing::error!("\r\nAttach error: {message}\r");
                    break;
                }
                Err(e) => {
                    tracing::error!("\r\nConnection error: {e}\r");
                    break;
                }
                _ => {
                    // Ignore other response types
                }
            }
        }
    });

    // Handle user input in main thread
    let stdin = io::stdin();
    let mut detach_sequence = DetachSequence::new();

    for key_result in stdin.keys() {
        match key_result {
            Ok(key) => {
                if detach_sequence.handle_key(key) {
                    tracing::info!("\r\nDetaching from container {container_id}\r");
                    // Send detach request
                    let detach_request = DaemonRequest::AttachDetach;
                    if let Err(e) = write_request(&mut stream_write, &detach_request).await {
                        tracing::error!("Error sending detach request: {e}");
                    }
                    break;
                }

                match key {
                    Key::Ctrl('c') => {
                        tracing::info!("\r\nExiting attach session\r");
                        // Send detach request
                        let detach_request = DaemonRequest::AttachDetach;
                        let _ = write_request(&mut stream_write, &detach_request).await;
                        break;
                    }
                    Key::Char(c) => {
                        // Send character to container
                        let input_request = DaemonRequest::AttachStdin {
                            data: vec![c as u8],
                        };
                        if let Err(e) = write_request(&mut stream_write, &input_request).await {
                            tracing::error!("\r\nError sending input: {e}\r");
                            break;
                        }
                    }
                    Key::Backspace => {
                        // Send backspace character
                        let input_request = DaemonRequest::AttachStdin {
                            data: vec![0x08], // ASCII backspace
                        };
                        if let Err(e) = write_request(&mut stream_write, &input_request).await {
                            tracing::error!("\r\nError sending backspace: {e}\r");
                            break;
                        }
                    }
                    Key::Ctrl(c) => {
                        // Send control character (except Ctrl+C and Ctrl+P which are handled above)
                        use tracing::debug;
                        let ctrl_byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
                        debug!(
                            "Forwarding control character Ctrl+{} (byte: 0x{:02x})",
                            c, ctrl_byte
                        );
                        let input_request = DaemonRequest::AttachStdin {
                            data: vec![ctrl_byte],
                        };
                        if let Err(e) = write_request(&mut stream_write, &input_request).await {
                            tracing::error!("\r\nError sending control character: {e}\r");
                            break;
                        }
                    }
                    _ => {
                        // Handle other keys as needed (arrows, function keys, etc.)
                        // For now, ignore them
                    }
                }
            }
            Err(e) => {
                tracing::error!("\r\nError reading input: {e}\r");
                break;
            }
        }
    }

    // Clean up
    output_task.abort();
    Ok(())
}

/// State machine for detecting Ctrl+P Ctrl+Q detach sequence
///
/// This implements the Docker-compatible detach sequence where users press
/// Ctrl+P followed by Ctrl+Q to gracefully detach from a container without
/// stopping it.
///
/// # State Transitions
///
/// - **Normal**: Initial state, waiting for Ctrl+P
///   - On Ctrl+P: transitions to `CtrlP` state
///   - On any other key: remains in `Normal` state
///
/// - **CtrlP**: Ctrl+P was pressed, waiting for Ctrl+Q
///   - On Ctrl+Q: detach sequence complete, returns `true`
///   - On any other key: resets to `Normal` state
/// ```
struct DetachSequence {
    state: DetachState,
}

#[derive(Debug, PartialEq, Copy, Clone)]
enum DetachState {
    Normal,
    CtrlP,
}

impl DetachSequence {
    fn new() -> Self {
        Self {
            state: DetachState::Normal,
        }
    }

    /// Handle a key press and return true if detach sequence is complete
    fn handle_key(&mut self, key: Key) -> bool {
        match (self.state, key) {
            (DetachState::Normal, Key::Ctrl('p')) => {
                self.state = DetachState::CtrlP;
                false
            }
            (DetachState::CtrlP, Key::Ctrl('q')) => {
                self.state = DetachState::Normal;
                true // Detach sequence detected
            }
            _ => {
                self.state = DetachState::Normal;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detach_sequence() {
        let mut seq = DetachSequence::new();

        // Normal keys should not trigger detach
        assert!(!seq.handle_key(Key::Char('a')));
        assert!(!seq.handle_key(Key::Ctrl('c')));

        // Ctrl+P alone should not trigger detach
        assert!(!seq.handle_key(Key::Ctrl('p')));
        // Reset state by pressing another key
        assert!(!seq.handle_key(Key::Char('x')));

        // Ctrl+P followed by Ctrl+Q should trigger detach
        assert!(!seq.handle_key(Key::Ctrl('p')));
        assert!(seq.handle_key(Key::Ctrl('q')));

        // Reset after detach
        assert!(!seq.handle_key(Key::Char('a')));

        // Ctrl+P followed by something else should not trigger detach
        assert!(!seq.handle_key(Key::Ctrl('p')));
        assert!(!seq.handle_key(Key::Char('a')));

        // Test another successful detach sequence
        assert!(!seq.handle_key(Key::Ctrl('p')));
        assert!(seq.handle_key(Key::Ctrl('q')));
    }
}
