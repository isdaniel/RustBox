use nix::{
    mount::{mount, umount2, MntFlags, MsFlags},
    sched::{unshare, CloneFlags},
    unistd::{chdir, chroot, execv, fork, ForkResult},
    sys::wait::waitpid,
};
use tracing::{error, info, warn};
use std::{
    ffi::CString,
    fs::{create_dir_all, read_to_string, remove_dir, write},
    path::{Path, PathBuf},
    process,
};

const CGROUP_BASE: &str = "/sys/fs/cgroup";

fn generate_cgroup_path() -> PathBuf {
    Path::new(CGROUP_BASE).join(format!("rustbox_{}", process::id()))
}

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub base_dir: String,
    pub memory_limit: String,
    pub shell_path: String,
    pub workdir: String,
    pub cpu_limit: String,
}

/// Move any remaining PIDs in cgroup back to root cgroup before removing directory.
fn drain_cgroup_and_remove(cgroup_path: &Path) {
    let procs_file = cgroup_path.join("cgroup.procs");
    if let Ok(contents) = read_to_string(&procs_file) {
        for pid_line in contents.lines().map(str::trim).filter(|s| !s.is_empty()) {
            if let Err(e) = write("/sys/fs/cgroup/cgroup.procs", pid_line) {
                warn!("Failed to move pid {} out of {}: {}", pid_line, cgroup_path.display(), e);
            } else {
                info!("Moved pid {} out of {}", pid_line, cgroup_path.display());
            }
        }
    }

    match remove_dir(cgroup_path) {
        Ok(_) => info!("Successfully removed cgroup: {}", cgroup_path.display()),
        Err(e) => warn!("Failed to remove cgroup {}: {}", cgroup_path.display(), e),
    }
}

fn parse_cpu_limit(cpu_limit_str: &str) -> Result<String, String> {
    let cpu_fraction: f64 = cpu_limit_str
        .parse()
        .map_err(|e| format!("Invalid CPU limit format '{}': {}", cpu_limit_str, e))?;

    if cpu_fraction <= 0.0 {
        return Err("CPU limit must be positive".to_string());
    }

    // cgroup v2 recommended period is 100000 microseconds (100ms)
    let period = 100_000u64;
    let quota = (cpu_fraction * period as f64) as u64;
    Ok(format!("{} {}", quota, period))
}

fn setup_cgroup(config: &SandboxConfig, cgroup_path: &Path) -> Result<(), String> {
    create_dir_all(cgroup_path).map_err(|e| format!("failed to create cgroup path: {}", e))?;

    write(cgroup_path.join("memory.max"), config.memory_limit.clone())
        .map_err(|e| format!("Failed to set memory limit: {}", e))?;

    let cpu_quota = parse_cpu_limit(&config.cpu_limit)?;
    write(cgroup_path.join("cpu.max"), cpu_quota)
        .map_err(|e| format!("Failed to set CPU limit: {}", e))?;

    write(
        cgroup_path.join("cgroup.procs"),
        process::id().to_string(),
    )
    .map_err(|e| format!("Failed to add process to cgroup: {}", e))?;

    Ok(())
}

fn ensure_dirs_exist(dirs: &[&Path]) -> Result<(), String> {
    for d in dirs {
        if !d.exists() {
            create_dir_all(d).map_err(|e| format!("Failed to create {}: {}", d.display(), e))?;
            info!("Created directory: {}", d.display());
        }
    }
    Ok(())
}

fn mount_overlay(lower: &Path, upper: &Path, work: &Path, merged: &Path) -> Result<(), String> {
    let opts = format!(
        "lowerdir={},upperdir={},workdir={}",
        lower.display(),
        upper.display(),
        work.display()
    );
    info!("Mounting overlay at {} with opts: {}", merged.display(), opts);
    mount(
        Some("overlay"),
        merged,
        Some("overlay"),
        MsFlags::empty(),
        Some(opts.as_str()),
    )
    .map_err(|e| format!("overlay mount failed: {}", e))?;
    Ok(())
}

#[inline]
fn umount_detach(path: &Path) {
    if let Err(e) = umount2(path, MntFlags::MNT_DETACH) {
        warn!("umount2 failed for {}: {}", path.display(), e);
    }
}

fn mount_proc_and_dev(merged: &Path) -> Result<(), String> {
    let proc_path = merged.join("proc");
    create_dir_all(&proc_path).map_err(|e| format!("Failed to create {}: {}", proc_path.display(), e))?;
    info!("Mounting /proc at: {}", proc_path.display());
    mount(Some("proc"), &proc_path, Some("proc"), MsFlags::empty(), None::<&str>)
        .map_err(|e| format!("mount /proc failed: {}", e))?;

    let dev_path = merged.join("dev");
    create_dir_all(&dev_path).map_err(|e| format!("Failed to create {}: {}", dev_path.display(), e))?;
    info!("Bind-mounting /dev at: {}", dev_path.display());
    mount(Some("/dev"), &dev_path, None::<&str>, MsFlags::MS_BIND | MsFlags::MS_REC, None::<&str>)
        .map_err(|e| format!("mount /dev failed: {}", e))?;

    Ok(())
}

fn run_in_namespace_and_wait(config: &SandboxConfig, merged: &Path) -> Result<(), String> {
    info!("Creating namespaces...");
    unshare(
        CloneFlags::CLONE_NEWNS
            | CloneFlags::CLONE_NEWPID
            | CloneFlags::CLONE_NEWUTS
            | CloneFlags::CLONE_NEWIPC
            | CloneFlags::CLONE_NEWNET
            | CloneFlags::CLONE_NEWUSER,
    )
    .map_err(|e| format!("unshare failed: {}", e))?;

    info!("Namespaces created successfully, forking inner process...");

    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            info!("(inner child) Setting up container environment in root: {}", merged.display());

            mount_proc_and_dev(merged)?;

            info!("(inner child) Changing root to: {}", merged.display());
            chroot(merged).map_err(|e| format!("chroot failed: {}", e))?;

            info!("(inner child) Changing working directory to: {}", config.workdir);
            chdir(config.workdir.as_str()).map_err(|e| format!("chdir failed: {}", e))?;

            info!("(inner child) Executing shell: {}", config.shell_path);
            let shell = CString::new(config.shell_path.clone()).map_err(|e| format!("Invalid shell path: {}", e))?;
            let arg0 = CString::new(config.shell_path.clone()).map_err(|e| format!("Invalid shell arg: {}", e))?;
            execv(&shell, &[arg0]).map_err(|e| format!("execv failed: {}", e))?;

            unreachable!("execv should not return");
        }
        Ok(ForkResult::Parent { child, .. }) => {
            info!("(inner parent) Waiting for inner child pid {}...", child);
            let _ = waitpid(child, None);

            info!("(inner parent) Cleaning up mounts inside namespace...");
            umount_detach(&merged.join("proc"));
            umount_detach(&merged.join("dev"));

            Ok(())
        }
        Err(e) => Err(format!("inner fork failed: {}", e)),
    }
}

pub fn run_sandbox(config: SandboxConfig) -> Result<(), String> {
    info!("Starting run_sandbox with config: {:?}", config);

    let lower = PathBuf::from(format!("{}/lowerdir", config.base_dir));
    let upper = PathBuf::from(format!("{}/upperdir", config.base_dir));
    let work = PathBuf::from(format!("{}/workdir", config.base_dir));
    let merged = PathBuf::from(format!("{}/merged", config.base_dir));

    ensure_dirs_exist(&[&lower, &upper, &work, &merged])?;

    mount_overlay(&lower, &upper, &work, &merged)?;

    let cgroup_path = generate_cgroup_path();
    info!("Setting up cgroups at: {}", cgroup_path.display());
    setup_cgroup(&config, &cgroup_path)?;

    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            info!("(namespaced parent) Calling unshare & inner fork...");
            match run_in_namespace_and_wait(&config, &merged) {
                Ok(_) => {
                    info!("(namespaced parent) Inner child finished, exiting namespaced parent.");
                    process::exit(0);
                }
                Err(e) => {
                    error!("(namespaced parent) Error: {}", e);
                    process::exit(1);
                }
            }
        }
        Ok(ForkResult::Parent { child, .. }) => {
            info!("(outer parent) Waiting for namespaced parent pid {}...", child);
            let _ = waitpid(child, None);

            info!("(outer parent) Cleaning up overlay mount at: {}", merged.display());
            umount_detach(&merged);

            info!("(outer parent) Draining and removing cgroup at: {}", cgroup_path.display());
            drain_cgroup_and_remove(&cgroup_path);

            info!("run_sandbox completed cleanup.");
            Ok(())
        }
        Err(e) => Err(format!("initial fork failed: {}", e)),
    }
}
