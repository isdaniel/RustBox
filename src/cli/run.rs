use crate::error::IpcError;
use crate::ipc::{DaemonRequest, DaemonResponse, IpcClient};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Create and start a new container")]
pub struct RunArgs {
    /// Optional container name (auto-generated if not provided)
    #[arg(long)]
    pub name: Option<String>,

    /// Memory limit (e.g., "256M", "1G")
    #[arg(long, default_value = "512M")]
    pub memory: String,

    /// CPU limit (e.g., "0.5", "1.0")
    #[arg(long, default_value = "1.0")]
    pub cpu: String,

    /// Working directory inside container
    #[arg(long, default_value = "/")]
    pub workdir: String,

    /// Path to rootfs directory
    #[arg(long, default_value = "./rootfs")]
    pub rootfs: String,

    /// Allocate a TTY for interactive use
    #[arg(long)]
    pub tty: bool,

    /// Isolate user namespace (CLONE_NEWUSER)
    #[arg(long)]
    pub isolate_user: bool,

    /// Isolate network namespace (CLONE_NEWNET)
    #[arg(long)]
    pub isolate_network: bool,

    /// Set environment variables (format: KEY=VALUE)
    #[arg(short = 'e', long = "env", value_name = "KEY=VALUE")]
    pub env: Vec<String>,

    /// Limit number of PIDs (processes) in the container
    #[arg(long)]
    pub pids_limit: Option<String>,

    /// CPU weight for fair scheduling (1-10000, default 100)
    #[arg(long)]
    pub cpu_weight: Option<String>,

    /// Memory+swap limit (e.g., "512M")
    #[arg(long)]
    pub memory_swap: Option<String>,

    /// Publish a container port to the host (e.g., -p 8080:80)
    #[arg(short = 'p', long = "publish")]
    pub publish: Vec<String>,

    /// Command to execute in the container
    #[arg(required = true, num_args = 1..)]
    pub command: Vec<String>,
}

pub async fn run_command(args: RunArgs) -> Result<(), IpcError> {
    let mut client = IpcClient::connect().await?;

    let request = DaemonRequest::RunRequest {
        name: args.name,
        memory_limit: args.memory,
        cpu_limit: args.cpu,
        command: args.command,
        workdir: args.workdir,
        rootfs_path: args.rootfs,
        tty: args.tty,
        isolate_user: args.isolate_user,
        isolate_network: args.isolate_network,
        env: args.env,
        pids_limit: args.pids_limit,
        cpu_weight: args.cpu_weight,
        memory_swap_limit: args.memory_swap,
        port_mappings: args.publish,
    };

    let response = client.send_request(request).await?;

    match response {
        DaemonResponse::RunResponse {
            container_id,
            name,
            state,
        } => {
            println!("{container_id}");
            println!("Container '{name}' started with state: {state}");
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
