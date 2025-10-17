use crate::error::IpcError;
use crate::ipc::{DaemonRequest, DaemonResponse, IpcClient};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Display detailed information about a container")]
pub struct InspectArgs {
    /// Container ID or name to inspect
    #[arg(required = true)]
    pub container: String,
}

pub async fn inspect_command(args: InspectArgs) -> Result<(), IpcError> {
    let mut client = IpcClient::connect().await?;

    let request = DaemonRequest::InspectRequest {
        container_id: args.container,
    };

    let response = client.send_request(request).await?;

    match response {
        DaemonResponse::InspectResponse { container } => {
            // Pretty-print the JSON with 2-space indentation
            let json = serde_json::to_string_pretty(&container)
                .map_err(|e| IpcError::InvalidFormat(e.to_string()))?;
            tracing::info!("{json}");
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
