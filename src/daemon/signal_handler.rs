use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;
use tracing::{error, info};

/// Setup signal handlers for graceful shutdown
pub async fn setup_signal_handlers(shutdown_tx: mpsc::Sender<()>) -> Result<(), std::io::Error> {
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    tokio::spawn(async move {
        tokio::select! {
            _ = sigterm.recv() => {
                info!("Received SIGTERM signal");
            }
            _ = sigint.recv() => {
                info!("Received SIGINT signal");
            }
        }

        info!("Initiating graceful shutdown...");
        if let Err(e) = shutdown_tx.send(()).await {
            error!("Failed to send shutdown signal: {}", e);
        }
    });

    Ok(())
}
