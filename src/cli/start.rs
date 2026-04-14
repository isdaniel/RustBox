use crate::error::IpcError;
use crate::ipc::{DaemonRequest, DaemonResponse, IpcClient};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Start a stopped container")]
pub struct StartArgs {
    /// Container ID or name to start
    #[arg(required = true)]
    pub container: String,
}

pub async fn start_command(args: StartArgs) -> Result<(), IpcError> {
    let mut client = IpcClient::connect().await?;

    let request = DaemonRequest::StartRequest {
        container_id: args.container,
    };

    let response = client.send_request(request).await?;

    match response {
        DaemonResponse::StartResponse {
            container_id,
            state,
        } => {
            println!("{container_id}");
            println!("Container started with state: {state}");
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
