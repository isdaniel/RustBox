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
#[ignore] // Ignore by default, run with: cargo test -- --ignored
async fn test_inspect_by_container_name() {
    let daemon = TestDaemon::start().await;
    
    // Create a container with a specific name
    let create_output = daemon.run_command(&[
        "run", 
        "--name", "test-web-server",
        "--memory", "128M", 
        "--cpu", "0.5", 
        "/bin/sleep", 
        "10"
    ]);
    
    assert!(create_output.status.success(), "Container creation should succeed");
    
    // Wait for container to start
    sleep(Duration::from_millis(500)).await;
    
    // Inspect container by name
    let inspect_output = daemon.run_command(&["inspect", "test-web-server"]);
    assert!(inspect_output.status.success(), "Inspect by name should succeed");
    
    let inspect_stdout = String::from_utf8_lossy(&inspect_output.stdout);
    assert!(inspect_stdout.contains("test-web-server"), "Inspect output should contain container name");
    assert!(inspect_stdout.contains("\"name\""), "Inspect output should be JSON format");
}

#[tokio::test]
#[ignore] // Ignore by default, run with: cargo test -- --ignored
async fn test_stop_by_container_name() {
    let daemon = TestDaemon::start().await;
    
    // Create a long-running container
    let create_output = daemon.run_command(&[
        "run", 
        "--name", "long-running-task",
        "--memory", "64M", 
        "--cpu", "0.25", 
        "/bin/sleep", 
        "300"
    ]);
    
    assert!(create_output.status.success(), "Container creation should succeed");
    
    // Wait for container to start
    sleep(Duration::from_millis(500)).await;
    
    // Stop container by name
    let stop_output = daemon.run_command(&["stop", "long-running-task"]);
    assert!(stop_output.status.success(), "Stop by name should succeed");
    
    let stop_stdout = String::from_utf8_lossy(&stop_output.stdout);
    assert!(stop_stdout.contains("stopped") || stop_stdout.contains("Stopped"), 
            "Stop output should indicate success");
}

#[tokio::test]
#[ignore] // Ignore by default, run with: cargo test -- --ignored
async fn test_remove_by_container_name() {
    let daemon = TestDaemon::start().await;
    
    // Create a container
    let create_output = daemon.run_command(&[
        "run", 
        "--name", "temporary-container",
        "--memory", "64M", 
        "--cpu", "0.25", 
        "/bin/echo", 
        "test"
    ]);
    
    assert!(create_output.status.success(), "Container creation should succeed");
    
    // Wait for container to finish
    sleep(Duration::from_secs(1)).await;
    
    // Remove container by name
    let remove_output = daemon.run_command(&["rm", "temporary-container"]);
    assert!(remove_output.status.success(), "Remove by name should succeed");
    
    // Verify container is gone
    let list_output = daemon.run_command(&["ps", "-a"]);
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    assert!(!list_stdout.contains("temporary-container"), 
            "Container should not appear in list after removal");
}

#[tokio::test]
#[ignore] // Ignore by default, run with: cargo test -- --ignored
async fn test_name_not_found_error() {
    let daemon = TestDaemon::start().await;
    
    // Try to inspect non-existent container by name
    let inspect_output = daemon.run_command(&["inspect", "nonexistent-container"]);
    assert!(!inspect_output.status.success(), "Inspect should fail for non-existent container");
    
    let inspect_stderr = String::from_utf8_lossy(&inspect_output.stderr);
    assert!(inspect_stderr.contains("not found") || inspect_stderr.contains("NotFound"), 
            "Error should indicate container not found");
}

#[tokio::test]
#[ignore] // Ignore by default, run with: cargo test -- --ignored
async fn test_both_id_and_name_work() {
    let daemon = TestDaemon::start().await;
    
    // Create a container with a name
    let create_output = daemon.run_command(&[
        "run", 
        "--name", "dual-access-test",
        "--memory", "64M", 
        "--cpu", "0.25", 
        "/bin/sleep", 
        "60"
    ]);
    
    assert!(create_output.status.success(), "Container creation should succeed");
    let create_stdout = String::from_utf8_lossy(&create_output.stdout);
    
    // Wait for container to start
    sleep(Duration::from_millis(500)).await;
    
    // Extract container ID from output (assuming format includes ID)
    // Note: This is a simplified extraction - adjust based on actual output format
    let lines: Vec<&str> = create_stdout.lines().collect();
    let container_id = lines.iter()
        .find(|line| line.len() == 12 || line.contains("container_id"))
        .map(|line| line.trim())
        .unwrap_or("");
    
    // Inspect by name
    let inspect_by_name = daemon.run_command(&["inspect", "dual-access-test"]);
    assert!(inspect_by_name.status.success(), "Inspect by name should succeed");
    
    // If we have a valid ID, try inspecting by ID
    if !container_id.is_empty() && container_id.len() >= 12 {
        let inspect_by_id = daemon.run_command(&["inspect", container_id]);
        assert!(inspect_by_id.status.success(), "Inspect by ID should also succeed");
        
        // Both outputs should be essentially the same
        let name_output = String::from_utf8_lossy(&inspect_by_name.stdout);
        let id_output = String::from_utf8_lossy(&inspect_by_id.stdout);
        
        assert!(name_output.contains("dual-access-test"), "Name-based inspect should show name");
        assert!(id_output.contains("dual-access-test"), "ID-based inspect should show name");
    }
}
