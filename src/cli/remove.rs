use crate::error::IpcError;
use crate::ipc::{DaemonRequest, DaemonResponse, IpcClient};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Remove a stopped container")]
pub struct RemoveArgs {
    /// Container ID or name to remove
    #[arg(required = true)]
    pub container: String,

    /// Force removal of running container
    #[arg(short, long)]
    pub force: bool,
}

pub async fn remove_command(args: RemoveArgs) -> Result<(), IpcError> {
    let mut client = IpcClient::connect().await?;

    let request = DaemonRequest::RemoveRequest {
        container_id: args.container,
        force: args.force,
    };

    let response = client.send_request(request).await?;

    match response {
        DaemonResponse::RemoveResponse {
            container_id,
            message,
        } => {
            tracing::info!("message: {message} container_id:{container_id}");
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
