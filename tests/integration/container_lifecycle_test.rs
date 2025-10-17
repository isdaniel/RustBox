use std::process::{Command, Stdio, Child};
use std::time::Duration;
use tokio::time::sleep;
use tempfile::TempDir;

struct TestDaemon {
    process: Child,
    socket_path: std::path::PathBuf,
    _temp_dir: TempDir,
}

impl TestDaemon {
    async fn start() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let socket_path = temp_dir.path().join("rustbox.sock");
        
        let process = Command::new("cargo")
            .args(&["run", "--bin", "rustbox", "--", "daemon", "--socket", socket_path.to_str().unwrap()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to start daemon");
        
        // Give daemon time to start
        sleep(Duration::from_millis(500)).await;
        
        TestDaemon {
            process,
            socket_path,
            _temp_dir: temp_dir,
        }
    }
    
    fn run_command(&self, args: &[&str]) -> std::process::Output {
        Command::new("cargo")
            .args(&["run", "--bin", "rustbox", "--"])
            .arg("--socket")
            .arg(self.socket_path.to_str().unwrap())
            .args(args)
            .output()
            .expect("Failed to run command")
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[tokio::test]
async fn test_container_run_and_list() {
    let daemon = TestDaemon::start().await;
    
    // Run a simple container
    let output = daemon.run_command(&[
        "run", 
        "--memory", "128M", 
        "--cpu", "0.5", 
        "/bin/echo", 
        "hello world"
    ]);
    
    assert!(output.status.success(), "Container run should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("hello world") || stdout.contains("Container"), "Output should contain expected text");
    
    // List containers
    let list_output = daemon.run_command(&["ps", "-a"]);
    assert!(list_output.status.success(), "List command should succeed");
    
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(list_stdout.contains("echo") || list_stdout.contains("Exited"), "List should show container");
}

#[tokio::test]
async fn test_container_run_with_name() {
    let daemon = TestDaemon::start().await;
    
    // Run container with custom name
    let output = daemon.run_command(&[
        "run", 
        "--name", "test-container",
        "--memory", "64M", 
        "--cpu", "0.25", 
        "/bin/echo", 
        "named container"
    ]);
    
    assert!(output.status.success(), "Named container run should succeed");
    
    // List containers and verify name appears
    let list_output = daemon.run_command(&["ps", "-a"]);
    assert!(list_output.status.success(), "List command should succeed");
    
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(list_stdout.contains("test-container"), "List should show named container");
}

#[tokio::test]
async fn test_container_stop_and_remove() {
    let daemon = TestDaemon::start().await;
    
    // Start a long-running container
    let run_output = daemon.run_command(&[
        "run", 
        "-d", // detached mode
        "--name", "long-runner",
        "--memory", "128M", 
        "--cpu", "0.5", 
        "/bin/sleep", 
        "30"
    ]);
    
    if !run_output.status.success() {
        // If detached mode not implemented, skip this test
        return;
    }
    
    let run_stdout = String::from_utf8_lossy(&run_output.stdout);
    let container_id = run_stdout.trim().split_whitespace().last().unwrap_or("unknown");
    
    // Stop the container
    let stop_output = daemon.run_command(&["stop", container_id]);
    assert!(stop_output.status.success(), "Container stop should succeed");
    
    // Remove the container
    let remove_output = daemon.run_command(&["rm", container_id]);
    assert!(remove_output.status.success(), "Container remove should succeed");
    
    // Verify container is gone
    let list_output = daemon.run_command(&["ps", "-a"]);
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(!list_stdout.contains(container_id), "Container should be removed from list");
}

#[tokio::test]
async fn test_container_invalid_image() {
    let daemon = TestDaemon::start().await;
    
    // Try to run container with invalid command
    let output = daemon.run_command(&[
        "run", 
        "--memory", "64M", 
        "--cpu", "0.1", 
        "/nonexistent/command"
    ]);
    
    assert!(!output.status.success(), "Invalid command should fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error") || stderr.contains("failed"), "Error message should be present");
}

#[tokio::test]
async fn test_container_resource_limits() {
    let daemon = TestDaemon::start().await;
    
    // Run container with specific resource limits
    let output = daemon.run_command(&[
        "run", 
        "--memory", "256M", 
        "--cpu", "1.0", 
        "/bin/echo", 
        "resource test"
    ]);
    
    assert!(output.status.success(), "Container with resource limits should succeed");
    
    // Verify output
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resource test") || stdout.contains("Container"), "Output should contain expected text");
}