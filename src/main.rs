use rustbox::{run_sandbox, SandboxConfig};

fn main() {
    match run_sandbox(SandboxConfig::default()) {
        Ok(_) => println!("Sandbox exited successfully."),
        Err(e) => eprintln!("Sandbox error: {}", e),
    }
}
