use crate::error::IpcError;
use crate::ipc::{DaemonRequest, DaemonResponse, IpcClient};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "List all containers")]
pub struct ListArgs {
    /// Show all containers (default shows just running)
    #[arg(short, long)]
    pub all: bool,
}

pub async fn list_command(args: ListArgs) -> Result<(), IpcError> {
    let mut client = IpcClient::connect().await?;

    let request = DaemonRequest::ListRequest { all: args.all };

    let response = client.send_request(request).await?;

    match response {
        DaemonResponse::ListResponse { containers } => {
            if containers.is_empty() {
                tracing::info!("No containers found");
                return Ok(());
            }

            // Print header
            tracing::info!(
                "{:<14} {:<20} {:<12} {:<10}",
                "CONTAINER ID", "NAME", "STATE", "COMMAND"
            );

            // Print each container
            for container in containers {
                let cmd = container.command.join(" ");
                let cmd_display = if cmd.len() > 30 {
                    format!("{}...", &cmd[..27])
                } else {
                    cmd
                };

                tracing::info!(
                    "{:<14} {:<20} {:<12} {:<10}",
                    &container.id[..12.min(container.id.len())],
                    container.name,
                    container.state,
                    cmd_display
                );
            }

            Ok(())
        }
        DaemonResponse::ErrorResponse { message, .. } => {
            tracing::error!("Error: {message}");
            std::process::exit(1);
        }
        _ => {
            tracing::error!("Unexpected response from daemon");
            std::process::exit(1);
        }
    }
}
