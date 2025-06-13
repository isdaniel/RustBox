use nix::{
    mount::{mount, umount2, MntFlags, MsFlags},
    sched::{unshare, CloneFlags},
    unistd::{chdir, chroot, execv, fork, ForkResult},
};
use std::{
    ffi::CString,
    fs::{create_dir_all, write},
    path::Path, process
};

const CGROUP_PATH : &str = "/sys/fs/cgroup/sandbox";

#[derive(Debug)]
pub struct SandboxConfig {
    pub base_dir: String,
    pub memory_limit: String,
    pub shell_path: String,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            base_dir: "./rootfs".to_string(),
            memory_limit: String::from("100M"),
            shell_path: "/bin/sh".to_string(),
        }
    }
}

pub fn run_sandbox(config: SandboxConfig) -> Result<(), String> {

    println!("config setting successfully, {:?}",config);

    let lower = format!("{}/lowerdir", config.base_dir);
    let upper = format!("{}/upperdir", config.base_dir);
    let work = format!("{}/workdir", config.base_dir);
    let merged = format!("{}/merged", config.base_dir);

    for dir in [&lower, &upper, &work, &merged] {
        if !Path::new(dir).exists() {
            create_dir_all(dir).map_err(|e| format!("mkdir {} failed: {}", dir, e))?;
        }
    }

    let overlay_opts = format!(
        "lowerdir={},upperdir={},workdir={}",
        lower, upper, work
    );

    mount(
        Some("overlay"),
        merged.as_str(),
        Some("overlay"),
        MsFlags::empty(),
        Some(overlay_opts.as_str()),
    )
    .map_err(|e| format!("Overlay mount failed: {}", e))?;

    // CGroup setup
    cgroup_setting(&config)?;

    // Namespace isolation
    unshare(
        CloneFlags::CLONE_NEWNS
            | CloneFlags::CLONE_NEWPID
            | CloneFlags::CLONE_NEWUTS
            | CloneFlags::CLONE_NEWIPC
            | CloneFlags::CLONE_NEWNET
            | CloneFlags::CLONE_NEWUSER,
    )
    .map_err(|e| format!("unshare failed: {}", e))?;

    println!("unshare executed successfully.");

    match unsafe { fork() } {
Ok(ForkResult::Child) => {
    let proc_path: String = format!("{}/proc", merged);
    mount(Some("proc"), proc_path.as_str(), Some("proc"), MsFlags::empty(), None::<&str>)
        .map_err(|e| format!("Mount /proc failed: {}", e))?;

    chroot(merged.as_str())
        .map_err(|e| format!("chroot failed: {}", e))?;
    chdir("/")
        .map_err(|e| format!("chdir failed: {}", e))?;

    let shell = CString::new(config.shell_path)
                        .map_err(|e: std::ffi::NulError| format!("Invalid shell path CString: {}", e))?;
    let arg0 = CString::new("sh")
        .map_err(|e| format!("Invalid arg0 CString: {}", e))?;
    execv(&shell, &[arg0])
        .map_err(|e| format!("execv failed: {}", e))?;
    println!("ForkResult::Child");
    Ok(())
}
Ok(ForkResult::Parent { child, .. }) => {
    let _ = nix::sys::wait::waitpid(child, None);
    let _ = umount2(merged.as_str(), MntFlags::MNT_DETACH);
    println!("ForkResult::Parent, child: {}, umount merged path {}",child, merged);
    Ok(())
}
Err(e) => Err(format!("fork failed: {}", e)),
    }
}

fn cgroup_setting(config: &SandboxConfig) -> Result<(), String> {
    create_dir_all(CGROUP_PATH).map_err(|e| e.to_string())?;
    write(format!("{}/memory.max", CGROUP_PATH), config.memory_limit.clone())
.map_err(|e| e.to_string())?;
    write(format!("{}/cgroup.procs", CGROUP_PATH), process::id().to_string())
    .map_err(|e| e.to_string())?;
    Ok(())
}
