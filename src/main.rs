use rustbox::{run_sandbox, SandboxConfig};

fn main() {
    let config = SandboxConfig {
        base_dir: "./rootfs".into(),
        memory_limit: String::from("100M"),
        shell_path: "/bin/sh".into(),
    };

    match run_sandbox(config) {
        Ok(_) => println!("Sandbox exited successfully."),
        Err(e) => eprintln!("Sandbox error: {}", e),
    }
}
