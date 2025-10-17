use crate::error::IpcError;
use crate::ipc::{DaemonRequest, DaemonResponse, IpcClient};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Stop a running container")]
pub struct StopArgs {
    /// Container ID or name to stop
    #[arg(required = true)]
    pub container: String,

    /// Timeout in seconds before forceful kill
    #[arg(short, long, default_value = "10")]
    pub timeout: u64,
}

pub async fn stop_command(args: StopArgs) -> Result<(), IpcError> {
    let mut client = IpcClient::connect().await?;

    let request = DaemonRequest::StopRequest {
        container_id: args.container,
        timeout: args.timeout,
    };

    let response = client.send_request(request).await?;

    match response {
        DaemonResponse::StopResponse {
            container_id,
            state,
        } => {
            tracing::info!("{container_id}");
            tracing::error!("Container stopped with state: {state}");
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
