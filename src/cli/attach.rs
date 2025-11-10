use crate::constants::{ESCAPE_BYTE, SOCKET_PATH};
use crate::error::IpcError;
use crate::ipc::{read_message, write_message, DaemonRequest, DaemonResponse};
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
    let mut stream = UnixStream::connect(SOCKET_PATH).await.map_err(|e| {
        tracing::error!(error = %e, "Failed to connect to daemon");
        IpcError::ConnectionFailed(e)
    })?;

    tracing::debug!("Connected to daemon, sending attach request");
    // Send initial attach request
    let request = DaemonRequest::AttachRequest {
        container_id: args.container_id,
    };

    write_message(&mut stream, &request).await?;
    let response = read_message(&mut stream).await?;

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
            streaming_attach_session(&container_id, stream).await?;

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

/// "echo $TERM" & "infocmp $TERM" commands often return escape codes for special keys, below mapping is based on xterm-256color.
/// basic VT 100 escape codes: https://espterm.github.io/docs/VT100%20escape%20codes.html
/// Convert a termion Key to raw bytes that should be sent to the PTY
fn key_to_bytes(key: Key) -> Vec<u8> {
    match key {
        // Regular character
        Key::Char('\n') => vec![b'\r'], // Convert newline to carriage return for TTY
        Key::Char(c) => vec![c as u8],

        // Control characters
        Key::Ctrl(c) => {
            // Convert Ctrl+letter to control code (Ctrl+A = 1, Ctrl+B = 2, etc.)

            // re-write ctrl + z => ctrl + c
            if c == 'z' {
                return vec![0x03];
            }

            let ctrl_byte = (c as u8).wrapping_sub(b'a').wrapping_add(1);
            vec![ctrl_byte]
        }

        // Alt characters (ESC followed by the character)
        Key::Alt(c) => vec![ESCAPE_BYTE, c as u8],

        // Special keys
        Key::Backspace => vec![0x08], // ASCII backspace (or 0x7F for some terminals)
        Key::Delete => vec![ESCAPE_BYTE, b'[', b'3', b'~'], // ESC[3~
        Key::Esc => vec![ESCAPE_BYTE], // ESC key
        Key::Null => vec![0x00],      // NULL byte

        // Arrow keys - ANSI escape sequences
        Key::Up => vec![ESCAPE_BYTE, b'[', b'A'],   // ESC[A
        Key::Down => vec![ESCAPE_BYTE, b'[', b'B'], // ESC[B
        Key::Right => vec![ESCAPE_BYTE, b'[', b'C'], // ESC[C
        Key::Left => vec![ESCAPE_BYTE, b'[', b'D'], // ESC[D

        // Modified arrow keys
        Key::ShiftUp => vec![ESCAPE_BYTE, b'[', b'1', b';', b'2', b'A'],
        Key::ShiftDown => vec![ESCAPE_BYTE, b'[', b'1', b';', b'2', b'B'],
        Key::ShiftRight => vec![ESCAPE_BYTE, b'[', b'1', b';', b'2', b'C'],
        Key::ShiftLeft => vec![ESCAPE_BYTE, b'[', b'1', b';', b'2', b'D'],

        Key::CtrlUp => vec![ESCAPE_BYTE, b'[', b'1', b';', b'5', b'A'],
        Key::CtrlDown => vec![ESCAPE_BYTE, b'[', b'1', b';', b'5', b'B'],
        Key::CtrlRight => vec![ESCAPE_BYTE, b'[', b'1', b';', b'5', b'C'],
        Key::CtrlLeft => vec![ESCAPE_BYTE, b'[', b'1', b';', b'5', b'D'],

        Key::AltUp => vec![ESCAPE_BYTE, b'[', b'1', b';', b'3', b'A'],
        Key::AltDown => vec![ESCAPE_BYTE, b'[', b'1', b';', b'3', b'B'],
        Key::AltRight => vec![ESCAPE_BYTE, b'[', b'1', b';', b'3', b'C'],
        Key::AltLeft => vec![ESCAPE_BYTE, b'[', b'1', b';', b'3', b'D'],

        // Navigation keys
        Key::Home => vec![ESCAPE_BYTE, b'O', b'H'], // ESC OH
        Key::End => vec![ESCAPE_BYTE, b'O', b'F'],  // ESC OF
        Key::PageUp => vec![ESCAPE_BYTE, b'[', b'5', b'~'], // ESC[5~
        Key::PageDown => vec![ESCAPE_BYTE, b'[', b'6', b'~'], // ESC[6~
        Key::Insert => vec![ESCAPE_BYTE, b'[', b'2', b'~'], // ESC[2~

        Key::CtrlHome => vec![ESCAPE_BYTE, b'[', b'1', b';', b'5', b'H'],
        Key::CtrlEnd => vec![ESCAPE_BYTE, b'[', b'1', b';', b'5', b'F'],

        // Function keys
        Key::F(1) => vec![ESCAPE_BYTE, b'O', b'P'], // ESC OP
        Key::F(2) => vec![ESCAPE_BYTE, b'O', b'Q'], // ESC OQ
        Key::F(3) => vec![ESCAPE_BYTE, b'O', b'R'], // ESC OR
        Key::F(4) => vec![ESCAPE_BYTE, b'O', b'S'], // ESC OS
        Key::F(5) => vec![ESCAPE_BYTE, b'[', b'1', b'5', b'~'], // ESC[15~
        Key::F(6) => vec![ESCAPE_BYTE, b'[', b'1', b'7', b'~'], // ESC[17~
        Key::F(7) => vec![ESCAPE_BYTE, b'[', b'1', b'8', b'~'], // ESC[18~
        Key::F(8) => vec![ESCAPE_BYTE, b'[', b'1', b'9', b'~'], // ESC[19~
        Key::F(9) => vec![ESCAPE_BYTE, b'[', b'2', b'0', b'~'], // ESC[20~
        Key::F(10) => vec![ESCAPE_BYTE, b'[', b'2', b'1', b'~'], // ESC[21~
        Key::F(11) => vec![ESCAPE_BYTE, b'[', b'2', b'3', b'~'], // ESC[23~
        Key::F(12) => vec![ESCAPE_BYTE, b'[', b'2', b'4', b'~'], // ESC[24~
        Key::F(n) => {
            // For other function keys, just ignore or send a placeholder
            tracing::warn!("Unsupported function key F{}", n);
            vec![]
        }

        // Tab keys
        Key::BackTab => vec![ESCAPE_BYTE, b'[', b'Z'], // ESC[Z (Shift+Tab)

        // This is safer than sending unknown sequences
        _ => {
            tracing::debug!("Ignoring unsupported key: {:?}", key);
            vec![]
        }
    }
}

/// Handle real streaming attach session with PTY forwarding
async fn streaming_attach_session(container_id: &str, stream: UnixStream) -> Result<(), IpcError> {
    tracing::info!(container_id = %container_id, "Starting streaming attach session");
    tracing::info!("Starting streaming attach to container: {container_id}");
    tracing::info!("Press Ctrl+C to detach");

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
            match read_message(&mut stream_read).await {
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

    for key_result in stdin.keys() {
        match key_result {
            Ok(key) => {
                // Handle Ctrl+C specially to exit
                if matches!(key, Key::Ctrl('c')) {
                    tracing::info!("\r\nExiting attach session\r");
                    // Send detach request
                    let detach_request = DaemonRequest::AttachDetach;
                    let _ = write_message(&mut stream_write, &detach_request).await;
                    break;
                }

                // Convert key to bytes and send to container
                let bytes = key_to_bytes(key);
                if !bytes.is_empty() {
                    let input_request = DaemonRequest::AttachStdin { data: bytes };
                    if let Err(e) = write_message(&mut stream_write, &input_request).await {
                        tracing::error!("\r\nError sending input: {e}\r");
                        break;
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
