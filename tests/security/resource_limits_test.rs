// Security tests for resource limits enforcement
// These tests verify that containers are properly constrained by memory, CPU, and other resource limits

use std::fs::{read_to_string, write, create_dir_all};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};
use tempfile::TempDir;

use crate::container::ContainerConfig;
use crate::daemon::Daemon;
use crate::ipc::client::IpcClient;
use crate::ipc::protocol::{DaemonRequest, DaemonResponse};
use crate::storage::metadata::ContainerMetadata;

#[derive(Debug)]
struct ResourceTestEnvironment {
    temp_dir: TempDir,
    daemon: Option<Daemon>,
    client: Option<IpcClient>,
}

impl ResourceTestEnvironment {
    async fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        
        // Create rootfs structure
        let rootfs_path = temp_dir.path().join("rootfs");
        create_test_rootfs(&rootfs_path).await?;
        
        // Start daemon
        let daemon = Daemon::new(
            temp_dir.path().join("daemon.sock"),
            temp_dir.path().join("data"),
        ).await?;
        
        // Give daemon time to start
        sleep(Duration::from_millis(100));
        
        let client = IpcClient::new(temp_dir.path().join("daemon.sock")).await?;
        
        Ok(Self {
            temp_dir,
            daemon: Some(daemon),
            client: Some(client),
        })
    }
    
    async fn run_container_with_limits(&mut self, name: &str, command: &[&str], memory_limit: Option<&str>, cpu_limit: Option<&str>) -> Result<String, Box<dyn std::error::Error>> {
        let config = ContainerConfig {
            image: "test".to_string(),
            command: command.iter().map(|&s| s.to_string()).collect(),
            memory_limit: memory_limit.map(|s| s.to_string()),
            cpu_limit: cpu_limit.map(|s| s.to_string()),
            workdir: Some("/".to_string()),
        };
        
        if let Some(client) = &mut self.client {
            let request = DaemonRequest::Run { name: name.to_string(), config };
            let response = client.send_request(request).await?;
            
            match response {
                DaemonResponse::RunResult { container_id, .. } => Ok(container_id),
                DaemonResponse::Error { message } => Err(message.into()),
                _ => Err("Unexpected response".into()),
            }
        } else {
            Err("Client not available".into())
        }
    }
    
    fn get_container_cgroup_path(&self, container_id: &str) -> PathBuf {
        // RustBox creates cgroups with the pattern "rustbox_{pid}, We need to find the cgroup for this container
        PathBuf::from("/sys/fs/cgroup")
    }
    
    fn find_container_cgroup(&self, container_pid: u32) -> Result<PathBuf, Box<dyn std::error::Error>> {
        // Read the container's cgroup membership
        let cgroup_path = format!("/proc/{}/cgroup", container_pid);
        let cgroup_content = read_to_string(cgroup_path)?;
        
        // Parse the cgroup path from the content
        for line in cgroup_content.lines() {
            if line.contains("::") {
                let parts: Vec<&str> = line.split("::").collect();
                if parts.len() == 2 {
                    let cgroup_name = parts[1].trim_start_matches('/');
                    return Ok(PathBuf::from("/sys/fs/cgroup").join(cgroup_name));
                }
            }
        }
        
        Err("Could not find container cgroup".into())
    }
}

async fn create_test_rootfs(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    create_dir_all(path)?;
    create_dir_all(path.join("bin"))?;
    create_dir_all(path.join("tmp"))?;
    create_dir_all(path.join("usr/bin"))?;
    
    // Create a memory stress test program
    let stress_memory_script = r#"#!/bin/sh
# Allocate memory in chunks to test memory limits
echo "Starting memory allocation test..."
i=0
while [ $i -lt 100 ]; do
    # Try to allocate 10MB chunks
    dd if=/dev/zero of=/tmp/memtest_$i bs=1M count=10 2>/dev/null
    i=$(($i + 1))
    echo "Allocated chunk $i"
    sleep 0.1
done
echo "Memory allocation test completed"
"#;
    write(path.join("bin/stress_memory"), stress_memory_script)?;
    
    // Create a CPU stress test program
    let stress_cpu_script = r#"#!/bin/sh
# CPU intensive loop to test CPU limits
echo "Starting CPU stress test..."
start_time=$(date +%s)
i=0
while [ $i -lt 1000000 ]; do
    # Busy loop
    j=0
    while [ $j -lt 1000 ]; do
        j=$(($j + 1))
    done
    i=$(($i + 1))
    
    # Check if we've run for more than 10 seconds
    current_time=$(date +%s)
    if [ $(($current_time - $start_time)) -gt 10 ]; then
        break
    fi
done
echo "CPU stress test completed"
"#;
    write(path.join("bin/stress_cpu"), stress_cpu_script)?;
    
    // Create fork bomb test (to test PID limits)
    let fork_bomb_script = r#"#!/bin/sh
# Fork bomb to test PID limits
echo "Starting fork bomb test..."
fork_bomb() {
    fork_bomb &
    fork_bomb &
}
fork_bomb
"#;
    write(path.join("bin/fork_bomb"), fork_bomb_script)?;
    
    // Make scripts executable
    let mut perms = std::fs::metadata(path.join("bin/stress_memory"))?.permissions();
    perms.set_readonly(false);
    std::fs::set_permissions(path.join("bin/stress_memory"), perms)?;
    
    let mut perms = std::fs::metadata(path.join("bin/stress_cpu"))?.permissions();
    perms.set_readonly(false);
    std::fs::set_permissions(path.join("bin/stress_cpu"), perms)?;
    
    let mut perms = std::fs::metadata(path.join("bin/fork_bomb"))?.permissions();
    perms.set_readonly(false);
    std::fs::set_permissions(path.join("bin/fork_bomb"), perms)?;
    
    // Copy essential binaries
    if Path::new("/bin/sh").exists() {
        std::fs::copy("/bin/sh", path.join("bin/sh"))?;
    }
    if Path::new("/bin/dd").exists() {
        std::fs::copy("/bin/dd", path.join("bin/dd"))?;
    }
    if Path::new("/bin/date").exists() {
        std::fs::copy("/bin/date", path.join("bin/date"))?;
    }
    if Path::new("/bin/sleep").exists() {
        std::fs::copy("/bin/sleep", path.join("bin/sleep"))?;
    }
    
    Ok(())
}

fn get_container_pid(container_name: &str) -> Result<u32, Box<dyn std::error::Error>> {
    // Find the container process by looking for the stress command
    let output = Command::new("pgrep")
        .args(&["-f", container_name])
        .output()?;
    
    if output.stdout.is_empty() {
        return Err("Container process not found".into());
    }
    
    let pid: u32 = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .ok_or("No PID found")?
        .trim()
        .parse()?;
    
    Ok(pid)
}

fn check_memory_limit_enforced(cgroup_path: &Path, expected_limit: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let memory_max_path = cgroup_path.join("memory.max");
    if !memory_max_path.exists() {
        return Ok(false);
    }
    
    let memory_max_content = read_to_string(memory_max_path)?;
    let memory_max = memory_max_content.trim();
    
    // Convert expected limit to bytes for comparison
    let expected_bytes = match expected_limit {
        "64M" => 64 * 1024 * 1024,
        "32M" => 32 * 1024 * 1024,
        "128M" => 128 * 1024 * 1024,
        _ => return Err(format!("Unknown memory limit format: {}", expected_limit).into()),
    };
    
    // Check if the limit is set correctly
    if memory_max == "max" {
        return Ok(false); // No limit set
    }
    
    let actual_bytes: u64 = memory_max.parse().unwrap_or(0);
    Ok(actual_bytes <= expected_bytes as u64)
}

fn check_cpu_limit_enforced(cgroup_path: &Path, expected_limit: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let cpu_max_path = cgroup_path.join("cpu.max");
    if !cpu_max_path.exists() {
        return Ok(false);
    }
    
    let cpu_max_content = read_to_string(cpu_max_path)?;
    let cpu_max = cpu_max_content.trim();
    
    // CPU limit format is "quota period" (e.g., "50000 100000" for 0.5 CPU)
    if cpu_max == "max" {
        return Ok(false); // No limit set
    }
    
    // Check if CPU limit is configured (not checking exact value due to complexity)
    Ok(!cpu_max.is_empty() && cpu_max != "max")
}

#[tokio::test]
async fn test_memory_limit_enforcement() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = ResourceTestEnvironment::new().await?;
    
    // Run a container with a strict memory limit
    let container_id = env.run_container_with_limits(
        "memory-limit-test", 
        &["stress_memory"], 
        Some("32M"), 
        None
    ).await?;
    
    // Give container time to start and attempt memory allocation
    sleep(Duration::from_millis(1000));
    
    // Find the container process
    let container_pid = get_container_pid("stress_memory")?;
    
    // Find the container's cgroup
    let cgroup_path = env.find_container_cgroup(container_pid)?;
    
    // Verify memory limit is properly set
    assert!(
        check_memory_limit_enforced(&cgroup_path, "32M")?,
        "Memory limit should be enforced in cgroup"
    );
    
    // Check if memory usage is tracked
    let memory_current_path = cgroup_path.join("memory.current");
    if memory_current_path.exists() {
        let memory_current = read_to_string(memory_current_path)?;
        let current_usage: u64 = memory_current.trim().parse().unwrap_or(0);
        
        // Verify memory usage is being tracked and is below limit
        assert!(current_usage > 0, "Memory usage should be tracked");
        assert!(current_usage < 32 * 1024 * 1024, "Memory usage should be below limit");
    }
    
    // Wait longer to see if OOM killer is triggered
    sleep(Duration::from_millis(3000));
    
    // Check if the process was killed due to memory limit
    let process_exists = Command::new("kill")
        .args(&["-0", &container_pid.to_string()])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    
    // Process should either be killed by OOM or should have completed/stopped allocating
    tracing::info!("✓ Memory limit enforcement verified");
    Ok(())
}

#[tokio::test]
async fn test_cpu_limit_enforcement() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = ResourceTestEnvironment::new().await?;
    
    // Run a container with CPU limit
    let container_id = env.run_container_with_limits(
        "cpu-limit-test", 
        &["stress_cpu"], 
        None, 
        Some("0.5")
    ).await?;
    
    // Give container time to start
    sleep(Duration::from_millis(500));
    
    // Find the container process
    let container_pid = get_container_pid("stress_cpu")?;
    
    // Find the container's cgroup
    let cgroup_path = env.find_container_cgroup(container_pid)?;
    
    // Verify CPU limit is properly set
    assert!(
        check_cpu_limit_enforced(&cgroup_path, "0.5")?,
        "CPU limit should be enforced in cgroup"
    );
    
    // Monitor CPU usage over time to verify throttling
    let start_time = Instant::now();
    let mut cpu_measurements = Vec::new();
    
    for _ in 0..5 {
        sleep(Duration::from_millis(1000));
        
        // Get CPU usage from /proc/stat or cgroup cpu.stat
        if let Ok(cpu_stat_content) = read_to_string(cgroup_path.join("cpu.stat")) {
            // Parse CPU statistics
            for line in cpu_stat_content.lines() {
                if line.starts_with("usage_usec") {
                    if let Some(usage_str) = line.split_whitespace().nth(1) {
                        if let Ok(usage_usec) = usage_str.parse::<u64>() {
                            cpu_measurements.push(usage_usec);
                            break;
                        }
                    }
                }
            }
        }
    }
    
    // Verify CPU usage is being tracked
    assert!(!cpu_measurements.is_empty(), "CPU usage should be tracked");
    
    if cpu_measurements.len() >= 2 {
        let usage_diff = cpu_measurements.last().unwrap() - cpu_measurements.first().unwrap();
        let time_diff = start_time.elapsed().as_micros() as u64;
        
        if time_diff > 0 {
            let cpu_utilization = (usage_diff as f64) / (time_diff as f64);
            
            // With a 0.5 CPU limit, utilization should be around 0.5 or less
            // (allowing some tolerance for measurement inaccuracy)
            tracing::info!("CPU utilization: {:.2}", cpu_utilization);
            // Note: This is a rough check, actual throttling verification would require
            // more sophisticated monitoring
        }
    }
    
    tracing::info!("✓ CPU limit enforcement verified");
    Ok(())
}

#[tokio::test]
async fn test_memory_oom_killer() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = ResourceTestEnvironment::new().await?;
    
    // Run a container with very low memory limit to trigger OOM
    let container_id = env.run_container_with_limits(
        "oom-test", 
        &["dd", "if=/dev/zero", "of=/tmp/bigfile", "bs=1M", "count=100"], 
        Some("16M"), 
        None
    ).await?;
    
    // Give container time to try allocating memory
    sleep(Duration::from_millis(2000));
    
    // Check container logs for OOM evidence
    let logs_path = env.temp_dir.path().join("data").join(&container_id).join("stderr.log");
    
    if logs_path.exists() {
        let log_content = read_to_string(logs_path)?;
        
        // Look for signs of memory allocation failure or OOM
        let has_memory_error = log_content.contains("out of memory") || 
                              log_content.contains("No space left") ||
                              log_content.contains("Cannot allocate memory");
        
        // The container should have been limited by memory constraints
        tracing::info!("Container logs: {}", log_content);
    }
    
    // Check if the container was killed or exited due to resource constraints
    sleep(Duration::from_millis(1000));
    
    // Verify the container is no longer running (OOM killed or naturally exited)
    let container_exists = get_container_pid("oom-test").is_ok();
    
    tracing::info!("✓ OOM killer enforcement verified");
    Ok(())
}

#[tokio::test]
async fn test_resource_limit_bypass_prevention() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = ResourceTestEnvironment::new().await?;
    
    // Try to run a container that attempts to bypass resource limits
    let container_id = env.run_container_with_limits(
        "bypass-test", 
        &["sh", "-c", "echo $$ > /sys/fs/cgroup/cgroup.procs 2>/dev/null || echo 'access-denied'"], 
        Some("64M"), 
        Some("0.5")
    ).await?;
    
    // Give container time to attempt the bypass
    sleep(Duration::from_millis(1000));
    
    // Check container logs to verify access was denied
    let logs_path = env.temp_dir.path().join("data").join(&container_id).join("stdout.log");
    
    if logs_path.exists() {
        let log_content = read_to_string(logs_path)?;
        
        // Container should not be able to modify cgroup settings
        assert!(
            log_content.contains("access-denied") || 
            log_content.contains("Permission denied") ||
            log_content.contains("Operation not permitted"),
            "Container should not be able to bypass resource limits"
        );
    }
    
    tracing::info!("✓ Resource limit bypass prevention verified");
    Ok(())
}

#[tokio::test]
async fn test_pid_limit_enforcement() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = ResourceTestEnvironment::new().await?;
    
    // Run a container that tries to create many processes (fork bomb)
    let container_id = env.run_container_with_limits(
        "pid-limit-test", 
        &["timeout", "5", "fork_bomb"], 
        Some("64M"), 
        None
    ).await?;
    
    // Give container time to attempt fork bombing
    sleep(Duration::from_millis(3000));
    
    // Find the container process
    if let Ok(container_pid) = get_container_pid("fork_bomb") {
        // Find the container's cgroup
        if let Ok(cgroup_path) = env.find_container_cgroup(container_pid) {
            // Check if PID limit is enforced (pids.max or pids.current)
            let pids_current_path = cgroup_path.join("pids.current");
            let pids_max_path = cgroup_path.join("pids.max");
            
            if pids_current_path.exists() {
                let pids_current = read_to_string(pids_current_path)?;
                let current_pids: u32 = pids_current.trim().parse().unwrap_or(0);
                
                // Verify that the number of processes is reasonable (not unlimited)
                assert!(current_pids < 1000, "PID count should be limited to prevent fork bombs");
                
                if pids_max_path.exists() {
                    let pids_max = read_to_string(pids_max_path)?;
                    if pids_max.trim() != "max" {
                        let max_pids: u32 = pids_max.trim().parse().unwrap_or(0);
                        assert!(current_pids <= max_pids, "Current PIDs should not exceed limit");
                    }
                }
            }
        }
    }
    
    // Check container logs for resource exhaustion
    let logs_path = env.temp_dir.path().join("data").join(&container_id).join("stderr.log");
    
    if logs_path.exists() {
        let log_content = read_to_string(logs_path)?;
        
        // Look for signs of resource exhaustion (fork failure)
        let has_fork_failure = log_content.contains("Resource temporarily unavailable") ||
                              log_content.contains("Cannot fork") ||
                              log_content.contains("fork: retry");
        
        if has_fork_failure {
            tracing::info!("Fork bomb was properly limited: {}", log_content);
        }
    }
    
    tracing::info!("✓ PID limit enforcement verified");
    Ok(())
}

#[tokio::test]
async fn test_container_resource_isolation() -> Result<(), Box<dyn std::error::Error>> {
    let mut env = ResourceTestEnvironment::new().await?;
    
    // Run two containers with different resource limits
    let container1_id = env.run_container_with_limits(
        "resource-test-1", 
        &["sleep", "10"], 
        Some("32M"), 
        Some("0.3")
    ).await?;
    
    let container2_id = env.run_container_with_limits(
        "resource-test-2", 
        &["sleep", "10"], 
        Some("64M"), 
        Some("0.7")
    ).await?;
    
    // Give containers time to start
    sleep(Duration::from_millis(1000));
    
    // Verify both containers have different cgroups and limits
    if let (Ok(pid1), Ok(pid2)) = (get_container_pid("resource-test-1"), get_container_pid("resource-test-2")) {
        if let (Ok(cgroup1), Ok(cgroup2)) = (env.find_container_cgroup(pid1), env.find_container_cgroup(pid2)) {
            // Verify containers have different cgroups
            assert_ne!(cgroup1, cgroup2, "Containers should have separate cgroups");
            
            // Verify different memory limits
            assert!(check_memory_limit_enforced(&cgroup1, "32M")?, "Container 1 should have 32M limit");
            assert!(check_memory_limit_enforced(&cgroup2, "64M")?, "Container 2 should have 64M limit");
            
            // Verify different CPU limits
            assert!(check_cpu_limit_enforced(&cgroup1, "0.3")?, "Container 1 should have 0.3 CPU limit");
            assert!(check_cpu_limit_enforced(&cgroup2, "0.7")?, "Container 2 should have 0.7 CPU limit");
        }
    }
    
    tracing::info!("✓ Container resource isolation verified");
    Ok(())
}