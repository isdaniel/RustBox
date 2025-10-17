use clap::{Parser, Subcommand};
use rustbox::cli::*;

#[derive(Parser, Debug)]
#[command(
    name = "rustbox",
    about = "A Docker-like container runtime written in Rust",
    version = "0.1.0"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create and start a new container
    Run(RunArgs),
    /// Stop a running container
    Stop(StopArgs),
    /// List containers
    #[command(alias = "ps")]
    List(ListArgs),
    /// Display detailed information about a container
    Inspect(InspectArgs),
    /// Remove a stopped container
    #[command(alias = "rm")]
    Remove(RemoveArgs),
    /// Fetch the logs of a container
    Logs(LogsArgs),
    /// Attach to a running container
    Attach(AttachArgs),
}

#[tokio::main]
async fn main() {
    // Initialize tracing subscriber for CLI output
    // Check if RUST_LOG is set for debug output, otherwise use clean format
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .init();
    } else {
        // Format without level prefix for cleaner output
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_target(false)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .with_level(false) // Hide the INFO/ERROR prefix
            .without_time()
            .init();
    }

    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Run(args) => run_command(args).await,
        Commands::Stop(args) => stop_command(args).await,
        Commands::List(args) => list_command(args).await,
        Commands::Inspect(args) => inspect_command(args).await,
        Commands::Remove(args) => remove_command(args).await,
        Commands::Logs(args) => logs::execute(args).await,
        Commands::Attach(args) => attach::execute(args).await,
    };

    if let Err(e) = result {
        tracing::error!("Error: {e}");
        std::process::exit(1);
    }
}
