use std::path::Path;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::container_manager::ContainerManager;
use rustbox::constants::SOCKET_PATH;
use rustbox::error::DaemonError;
use rustbox::ipc::{read_message, write_message, DaemonRequest, DaemonResponse};

pub struct DaemonServer {
    listener: UnixListener,
    container_manager: ContainerManager,
    shutdown_rx: mpsc::Receiver<()>,
}

impl DaemonServer {
    /// Create a new daemon server
    pub async fn new(shutdown_rx: mpsc::Receiver<()>) -> Result<Self, DaemonError> {
        // Remove existing socket if it exists
        let socket_path = Path::new(SOCKET_PATH);
        if socket_path.exists() {
            std::fs::remove_file(socket_path).map_err(DaemonError::SocketBind)?;
        }

        // Bind to Unix socket
        info!("Binding to socket: {}", SOCKET_PATH);
        let listener = UnixListener::bind(SOCKET_PATH).map_err(DaemonError::SocketBind)?;

        // Set socket permissions (0660)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(SOCKET_PATH).map_err(DaemonError::SocketBind)?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o660);
            std::fs::set_permissions(SOCKET_PATH, permissions).map_err(DaemonError::SocketBind)?;
        }

        let container_manager = ContainerManager::new();

        // Recover container state from disk
        container_manager.recover_state().await?;

        Ok(Self {
            listener,
            container_manager,
            shutdown_rx,
        })
    }

    /// Run the server event loop
    pub async fn run(mut self) -> Result<(), DaemonError> {
        info!("Daemon server started, listening on {}", SOCKET_PATH);

        loop {
            tokio::select! {
                // Handle incoming connections
                result = self.listener.accept() => {
                    match result {
                        Ok((stream, _addr)) => {
                            let container_manager = self.container_manager.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_connection(stream, container_manager).await {
                                    error!("Error handling connection: {}", e);
                                }
                            });
                        }
                        Err(e) => {
                            error!("Failed to accept connection: {}", e);
                        }
                    }
                }

                // Handle shutdown signal
                _ = self.shutdown_rx.recv() => {
                    info!("Received shutdown signal, stopping server...");
                    self.shutdown().await;
                    break;
                }
            }
        }

        info!("Daemon server stopped");
        Ok(())
    }

    /// Graceful shutdown
    async fn shutdown(&self) {
        info!("Starting graceful shutdown...");

        // Stop all containers with 30-second timeout
        self.container_manager.stop_all_containers(30).await;

        // Remove socket file
        if let Err(e) = std::fs::remove_file(SOCKET_PATH) {
            warn!("Failed to remove socket file: {}", e);
        }

        info!("Graceful shutdown complete");
    }
}

/// Handle a single client connection
async fn handle_connection(
    mut stream: UnixStream,
    container_manager: ContainerManager,
) -> Result<(), DaemonError> {
    // Read request
    let request: DaemonRequest = read_message(&mut stream).await.map_err(DaemonError::Ipc)?;

    info!("Received request: {:?}", request);

    // Process request
    let response = process_request(request.clone(), &container_manager).await;

    // Write response
    write_message(&mut stream, &response)
        .await
        .map_err(DaemonError::Ipc)?;

    info!("Sent response: {:?}", response);

    // Check if we need to transition to streaming attach mode
    if let (DaemonRequest::AttachRequest { container_id }, DaemonResponse::AttachResponse { .. }) =
        (&request, &response)
    {
        info!(
            "Transitioning to streaming attach mode for container: {}",
            container_id
        );

        // Hand over the connection to streaming attach handler
        if let Err(e) = container_manager
            .handle_streaming_attach(container_id.clone(), stream)
            .await
        {
            error!("Streaming attach failed: {}", e);
        }
    }

    Ok(())
}

/// Process a daemon request and generate response
async fn process_request(
    request: DaemonRequest,
    container_manager: &ContainerManager,
) -> DaemonResponse {
    match request {
        DaemonRequest::RunRequest {
            name,
            memory_limit,
            cpu_limit,
            command,
            workdir,
            rootfs_path,
            tty,
        } => {
            let config = rustbox::container::ContainerConfig {
                memory_limit,
                cpu_limit,
                command,
                workdir,
                rootfs_path,
                tty,
            };

            match container_manager.handle_run(name, config).await {
                Ok(response) => response,
                Err(e) => DaemonResponse::ErrorResponse {
                    message: e.to_string(),
                    code: 1,
                },
            }
        }

        DaemonRequest::StopRequest {
            container_id,
            timeout,
        } => match container_manager.handle_stop(container_id, timeout).await {
            Ok(response) => response,
            Err(e) => DaemonResponse::ErrorResponse {
                message: e.to_string(),
                code: 1,
            },
        },

        DaemonRequest::ListRequest { all } => match container_manager.handle_list(all).await {
            Ok(response) => response,
            Err(e) => DaemonResponse::ErrorResponse {
                message: e.to_string(),
                code: 1,
            },
        },

        DaemonRequest::InspectRequest { container_id } => {
            match container_manager.handle_inspect(container_id).await {
                Ok(response) => response,
                Err(e) => DaemonResponse::ErrorResponse {
                    message: e.to_string(),
                    code: 1,
                },
            }
        }

        DaemonRequest::RemoveRequest {
            container_id,
            force,
        } => match container_manager.handle_remove(container_id, force).await {
            Ok(response) => response,
            Err(e) => DaemonResponse::ErrorResponse {
                message: e.to_string(),
                code: 1,
            },
        },

        DaemonRequest::StatusRequest => match container_manager.handle_status().await {
            Ok(response) => response,
            Err(e) => DaemonResponse::ErrorResponse {
                message: e.to_string(),
                code: 1,
            },
        },

        DaemonRequest::LogsRequest { container_id, tail } => {
            match container_manager.handle_logs(container_id, tail).await {
                Ok(response) => response,
                Err(e) => DaemonResponse::ErrorResponse {
                    message: e.to_string(),
                    code: 1,
                },
            }
        }

        DaemonRequest::AttachRequest { container_id } => {
            // Attach functionality is delegated to container manager
            // The actual PTY forwarding implementation is pending
            match container_manager.handle_attach(container_id).await {
                Ok(response) => response,
                Err(e) => DaemonResponse::ErrorResponse {
                    message: e.to_string(),
                    code: 1,
                },
            }
        }
        // Streaming attach messages - these will be handled separately in streaming mode
        DaemonRequest::AttachStdin { .. } => DaemonResponse::ErrorResponse {
            message: "AttachStdin should be handled in streaming mode".to_string(),
            code: 1,
        },
        DaemonRequest::AttachDetach => DaemonResponse::ErrorResponse {
            message: "AttachDetach should be handled in streaming mode".to_string(),
            code: 1,
        },
    }
}
