use crate::error::IpcError;
use crate::ipc::{DaemonRequest, DaemonResponse, IpcClient};
use clap::Args;

#[derive(Args, Debug)]
pub struct LogsArgs {
    /// Container ID or name
    pub container_id: String,

    /// Number of lines to show from the end of the logs
    #[arg(short = 'n', long, default_value = "100")]
    pub tail: usize,
}

pub async fn execute(args: LogsArgs) -> Result<(), IpcError> {
    let mut client = IpcClient::connect().await?;

    let request = DaemonRequest::LogsRequest {
        container_id: args.container_id,
        tail: args.tail,
    };

    let response = client.send_request(request).await?;

    match response {
        DaemonResponse::LogsResponse {
            container_id,
            stdout,
            stderr,
        } => {
            let has_stdout = !stdout.is_empty();
            let has_stderr = !stderr.is_empty();

            if has_stdout {
                tracing::info!("==> stdout <==");
                for line in stdout {
                    tracing::info!("{line}");
                }
            }

            if has_stderr {
                tracing::info!("==> stderr <==");
                for line in stderr {
                    tracing::error!("{line}");
                }
            }

            if !has_stdout && !has_stderr {
                tracing::info!("No logs available for container: {container_id}");
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
