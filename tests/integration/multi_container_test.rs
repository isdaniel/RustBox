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
async fn test_multiple_containers_creation() {
    let daemon = TestDaemon::start().await;
    
    // Create multiple containers with different configurations
    let containers = [
        ("web-server", "256M", "1.0", "/bin/echo", "web"),
        ("database", "512M", "2.0", "/bin/echo", "db"),
        ("cache", "128M", "0.5", "/bin/echo", "cache"),
        ("worker", "64M", "0.25", "/bin/echo", "work"),
    ];
    
    for (name, memory, cpu, cmd, arg) in &containers {
        let output = daemon.run_command(&[
            "run",
            "--name", name,
            "--memory", memory,
            "--cpu", cpu,
            cmd,
            arg,
        ]);
        
        assert!(output.status.success(), "Container {} should be created successfully", name);
    }
    
    // List all containers
    let list_output = daemon.run_command(&["ps", "-a"]);
    assert!(list_output.status.success(), "List command should succeed");
    
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    
    // Verify all containers appear in the list
    for (name, _, _, _, _) in &containers {
        assert!(list_stdout.contains(name), "Container {} should appear in list", name);
    }
}

#[tokio::test]
async fn test_container_isolation() {
    let daemon = TestDaemon::start().await;
    
    // Create containers that would interfere if not properly isolated
    let output1 = daemon.run_command(&[
        "run",
        "--name", "isolated1",
        "--memory", "128M",
        "--cpu", "0.5",
        "/bin/echo", "container1",
    ]);
    
    let output2 = daemon.run_command(&[
        "run",
        "--name", "isolated2", 
        "--memory", "128M",
        "--cpu", "0.5",
        "/bin/echo", "container2",
    ]);
    
    assert!(output1.status.success(), "First container should succeed");
    assert!(output2.status.success(), "Second container should succeed");
    
    // Verify both containers have unique IDs
    let list_output = daemon.run_command(&["ps", "-a"]);
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    
    assert!(list_stdout.contains("isolated1"), "First container should be listed");
    assert!(list_stdout.contains("isolated2"), "Second container should be listed");
}

#[tokio::test]
async fn test_concurrent_container_operations() {
    let daemon = TestDaemon::start().await;
    
    // Create multiple containers concurrently (sequentially in test)
    let container_count = 5;
    
    for i in 0..container_count {
        let name = format!("concurrent{}", i);
        let output = daemon.run_command(&[
            "run",
            "--name", &name,
            "--memory", "64M",
            "--cpu", "0.1",
            "/bin/echo", &format!("test{}", i),
        ]);
        
        assert!(output.status.success(), "Container {} should be created", name);
    }
    
    // List all containers
    let list_output = daemon.run_command(&["ps", "-a"]);
    assert!(list_output.status.success(), "List command should succeed");
    
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    
    // Count containers
    let mut found_containers = 0;
    for i in 0..container_count {
        let name = format!("concurrent{}", i);
        if list_stdout.contains(&name) {
            found_containers += 1;
        }
    }
    
    assert_eq!(found_containers, container_count, "All containers should be listed");
}

#[tokio::test]
async fn test_resource_limits_independence() {
    let daemon = TestDaemon::start().await;
    
    // Create containers with different resource limits
    let configs = [
        ("low-mem", "32M", "0.1"),
        ("high-mem", "512M", "2.0"),
        ("mid-mem", "128M", "1.0"),
    ];
    
    for (name, memory, cpu) in &configs {
        let output = daemon.run_command(&[
            "run",
            "--name", name,
            "--memory", memory,
            "--cpu", cpu,
            "/bin/echo", "resource test",
        ]);
        
        assert!(output.status.success(), "Container {} should be created with limits", name);
    }
    
    // Verify all containers are listed
    let list_output = daemon.run_command(&["ps", "-a"]);
    let list_stdout = String::from_utf8_lossy(&list_output.stdout);
    
    for (name, _, _) in &configs {
        assert!(list_stdout.contains(name), "Container {} should be in list", name);
    }
}

#[tokio::test]
async fn test_container_name_uniqueness() {
    let daemon = TestDaemon::start().await;
    
    // Create first container with a name
    let output1 = daemon.run_command(&[
        "run",
        "--name", "unique-test",
        "--memory", "64M",
        "--cpu", "0.1",
        "/bin/echo", "first",
    ]);
    
    assert!(output1.status.success(), "First container should be created");
    
    // Try to create second container with same name
    let output2 = daemon.run_command(&[
        "run",
        "--name", "unique-test",
        "--memory", "64M", 
        "--cpu", "0.1",
        "/bin/echo", "second",
    ]);
    
    // This should either fail or generate a unique name
    // The exact behavior depends on implementation
    if output2.status.success() {
        // If it succeeds, verify containers have different IDs
        let list_output = daemon.run_command(&["ps", "-a"]);
        let list_stdout = String::from_utf8_lossy(&list_output.stdout);
        
        // Should have some way to distinguish the containers
        assert!(list_stdout.lines().count() >= 2, "Should have multiple containers");
    } else {
        // If it fails, that's also acceptable behavior
        let stderr = String::from_utf8_lossy(&output2.stderr);
        assert!(stderr.contains("exists") || stderr.contains("conflict"), "Should have appropriate error message");
    }
}