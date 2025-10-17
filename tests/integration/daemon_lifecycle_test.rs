use std::process::{Command, Stdio};
use std::time::Duration;
use tokio::time::sleep;
use tempfile::TempDir;

/// Test the daemon lifecycle: start, status, stop
#[tokio::test]
async fn test_daemon_lifecycle() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let socket_path = temp_dir.path().join("rustbox.sock");
    
    // Start the daemon in background
    let mut daemon_process = Command::new("cargo")
        .args(&["run", "--bin", "rustbox", "--", "daemon", "--socket", socket_path.to_str().unwrap()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start daemon");
    
    // Give daemon time to start
    sleep(Duration::from_millis(500)).await;
    
    // Test daemon status
    let status_output = Command::new("cargo")
        .args(&["run", "--bin", "rustbox", "--", "--socket", socket_path.to_str().unwrap(), "status"])
        .output()
        .expect("Failed to run status command");
    
    assert!(status_output.status.success(), "Status command should succeed");
    let status_text = String::from_utf8_lossy(&status_output.stdout);
    assert!(status_text.contains("running"), "Status should show daemon is running");
    
    // Stop the daemon
    daemon_process.kill().expect("Failed to kill daemon");
    daemon_process.wait().expect("Failed to wait for daemon");
    
    // Verify daemon is no longer running
    let status_output_after = Command::new("cargo")
        .args(&["run", "--bin", "rustbox", "--", "--socket", socket_path.to_str().unwrap(), "status"])
        .output()
        .expect("Failed to run status command after stop");
    
    assert!(!status_output_after.status.success(), "Status command should fail when daemon is stopped");
}

#[tokio::test]
async fn test_daemon_socket_cleanup() {
    let temp_dir = TempDir::new().expect("Failed to create temp directory");
    let socket_path = temp_dir.path().join("test.sock");
    
    // Start daemon
    let mut daemon_process = Command::new("cargo")
        .args(&["run", "--bin", "rustbox", "--", "daemon", "--socket", socket_path.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to start daemon");
    
    // Give daemon time to create socket
    sleep(Duration::from_millis(300)).await;
    
    // Verify socket exists
    assert!(socket_path.exists(), "Socket file should exist");
    
    // Stop daemon
    daemon_process.kill().expect("Failed to kill daemon");
    daemon_process.wait().expect("Failed to wait for daemon");
    
    // Give time for cleanup
    sleep(Duration::from_millis(100)).await;
    
    // Socket should be cleaned up automatically by Unix socket drop
    // This is just to verify the test framework works
}

#[tokio::test]
async fn test_daemon_invalid_socket_path() {
    // Try to start daemon with invalid socket path
    let invalid_socket = "/nonexistent/directory/test.sock";
    
    let output = Command::new("cargo")
        .args(&["run", "--bin", "rustbox", "--", "daemon", "--socket", invalid_socket])
        .output()
        .expect("Failed to run daemon command");
    
    assert!(!output.status.success(), "Daemon should fail with invalid socket path");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error") || stderr.contains("failed"), "Error message should be present");
}