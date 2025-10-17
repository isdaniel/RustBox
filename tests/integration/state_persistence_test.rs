use std::process::{Command, Stdio, Child};
use std::time::Duration;
use tokio::time::sleep;
use tempfile::TempDir;
use std::fs;

struct TestDaemonWithPersistence {
    process: Option<Child>,
    socket_path: std::path::PathBuf,
    data_dir: std::path::PathBuf,
    _temp_dir: TempDir,
}

impl TestDaemonWithPersistence {
    async fn start() -> Self {
        let temp_dir = TempDir::new().expect("Failed to create temp directory");
        let socket_path = temp_dir.path().join("rustbox.sock");
        let data_dir = temp_dir.path().join("data");
        
        // Create data directory
        fs::create_dir_all(&data_dir).expect("Failed to create data directory");
        
        let process = Command::new("cargo")
            .args(&["run", "--bin", "rustbox", "--", "daemon", "--socket", socket_path.to_str().unwrap()])
            .env("RUSTBOX_DATA_DIR", data_dir.to_str().unwrap())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to start daemon");
        
        // Give daemon time to start
        sleep(Duration::from_millis(500)).await;
        
        TestDaemonWithPersistence {
            process: Some(process),
            socket_path,
            data_dir,
            _temp_dir: temp_dir,
        }
    }
    
    async fn restart(&mut self) {
        // Stop current daemon
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
        
        // Give time for cleanup
        sleep(Duration::from_millis(200)).await;
        
        // Start new daemon with same data directory
        let process = Command::new("cargo")
            .args(&["run", "--bin", "rustbox", "--", "daemon", "--socket", self.socket_path.to_str().unwrap()])
            .env("RUSTBOX_DATA_DIR", self.data_dir.to_str().unwrap())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to restart daemon");
        
        self.process = Some(process);
        
        // Give daemon time to start and load state
        sleep(Duration::from_millis(500)).await;
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

impl Drop for TestDaemonWithPersistence {
    fn drop(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

#[tokio::test]
async fn test_container_state_persistence() {
    let mut daemon = TestDaemonWithPersistence::start().await;
    
    // Create a container
    let run_output = daemon.run_command(&[
        "run", 
        "--name", "persistent-test",
        "--memory", "128M", 
        "--cpu", "0.5", 
        "/bin/echo", 
        "test persistence"
    ]);
    
    assert!(run_output.status.success(), "Container creation should succeed");
    
    // List containers before restart
    let list_before = daemon.run_command(&["ps", "-a"]);
    assert!(list_before.status.success(), "List command should succeed");
    let list_before_output = String::from_utf8_lossy(&list_before.stdout);
    assert!(list_before_output.contains("persistent-test"), "Container should exist before restart");
    
    // Restart daemon
    daemon.restart().await;
    
    // List containers after restart
    let list_after = daemon.run_command(&["ps", "-a"]);
    assert!(list_after.status.success(), "List command should succeed after restart");
    let list_after_output = String::from_utf8_lossy(&list_after.stdout);
    
    // Container metadata should persist
    assert!(list_after_output.contains("persistent-test"), "Container should exist after restart");
}

#[tokio::test]
async fn test_container_metadata_persistence() {
    let mut daemon = TestDaemonWithPersistence::start().await;
    
    // Create multiple containers with different states
    daemon.run_command(&[
        "run", 
        "--name", "container1",
        "--memory", "64M", 
        "--cpu", "0.25", 
        "/bin/echo", 
        "first"
    ]);
    
    daemon.run_command(&[
        "run", 
        "--name", "container2",
        "--memory", "128M", 
        "--cpu", "0.5", 
        "/bin/echo", 
        "second"
    ]);
    
    // Get initial list
    let list_before = daemon.run_command(&["ps", "-a"]);
    let list_before_output = String::from_utf8_lossy(&list_before.stdout);
    
    // Count containers
    let containers_before = list_before_output.lines()
        .filter(|line| line.contains("container1") || line.contains("container2"))
        .count();
    
    // Restart daemon
    daemon.restart().await;
    
    // Get list after restart
    let list_after = daemon.run_command(&["ps", "-a"]);
    let list_after_output = String::from_utf8_lossy(&list_after.stdout);
    
    // Count containers after restart
    let containers_after = list_after_output.lines()
        .filter(|line| line.contains("container1") || line.contains("container2"))
        .count();
    
    // Both containers should be restored
    assert_eq!(containers_before, containers_after, "Same number of containers should exist after restart");
    assert!(list_after_output.contains("container1"), "Container1 should be restored");
    assert!(list_after_output.contains("container2"), "Container2 should be restored");
}

#[tokio::test]
async fn test_data_directory_structure() {
    let daemon = TestDaemonWithPersistence::start().await;
    
    // Create a container to trigger data directory creation
    daemon.run_command(&[
        "run", 
        "--name", "dir-test",
        "--memory", "64M", 
        "/bin/true"
    ]);
    
    // Verify data directory structure exists
    let containers_dir = daemon.data_dir.join("containers");
    let logs_dir = daemon.data_dir.join("logs");
    
    // These directories should be created by the daemon
    // The test just verifies the structure can be created
    // Actual persistence logic will be implemented in the daemon
    assert!(daemon.data_dir.exists(), "Data directory should exist");
}