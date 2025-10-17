mod container_manager;
mod pid_lock;
mod server;
mod signal_handler;

use pid_lock::PidLock;
use server::DaemonServer;
use signal_handler::setup_signal_handlers;
use std::process;
use tokio::sync::mpsc;
use tracing::{error, info};
use tracing_subscriber::{filter::EnvFilter, fmt};

#[tokio::main]
async fn main() {
    // Initialize logging with RUST_LOG environment variable
    // Default to info level if not set
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    fmt().with_env_filter(env_filter).with_target(false).init();

    info!("Starting RustBox daemon...");

    // Try to acquire daemon lock to ensure only one instance is running
    let pid_lock = PidLock::new();
    if let Err(e) = pid_lock.acquire() {
        error!("Failed to acquire daemon lock: {}", e);
        error!("Another daemon instance may already be running.");
        error!("If you're sure no other instance is running, remove the PID file manually:");
        error!("sudo rm {}", rustbox::constants::PID_FILE_PATH);
        process::exit(1);
    }

    // Create shutdown channel
    let (shutdown_tx, shutdown_rx) = mpsc::channel(1);

    // Setup signal handlers for graceful shutdown
    if let Err(e) = setup_signal_handlers(shutdown_tx).await {
        error!("Failed to setup signal handlers: {}", e);
        process::exit(1);
    }

    // Create and run the daemon server
    match DaemonServer::new(shutdown_rx).await {
        Ok(server) => {
            info!("RustBox daemon started successfully");
            if let Err(e) = server.run().await {
                error!("Daemon server error: {}", e);
                // PidLock will be automatically released when dropped
                process::exit(1);
            }
            info!("RustBox daemon shutdown complete");
            process::exit(0);
        }
        Err(e) => {
            error!("Failed to start daemon server: {}", e);
            // PidLock will be automatically released when dropped
            process::exit(1);
        }
    }
}
