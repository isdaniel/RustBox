use clap::Parser;
use rustbox::{run_sandbox, SandboxConfig};
use tracing_subscriber;

#[derive(Parser, Debug)]
#[command(
    name = "rustbox",
    about = "A Docker-like container runtime written in Rust",
    version = "0.1.0"
)]
struct Cli {
    /// Base directory for the container filesystem
    #[arg(long, default_value = "./rootfs")]
    base_dir: String,

    /// Memory limit (e.g., "128M")
    #[arg(long, default_value = "128M")]
    memory: String,

    /// CPU limit as fraction of one core (e.g., "0.5" for 50% of one core)
    #[arg(long, default_value = "1.0")]
    cpu_limit: String,

    /// Shell to execute
    #[arg(long, default_value = "/bin/sh")]
    shell: String,

    /// Working directory
    #[arg(long, default_value = "/")]
    workdir: String,
}


fn main() -> Result<(), String> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let cli = Cli::parse();
    let config = SandboxConfig {
        base_dir: cli.base_dir,
        memory_limit: cli.memory,
        cpu_limit: cli.cpu_limit,
        shell_path: cli.shell,
        workdir: cli.workdir,
    };

    run_sandbox(config).map_err(|e| e.to_string())?;

    Ok(())
}