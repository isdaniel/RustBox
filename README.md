# Rustbox

> A lightweight sandboxing utility written in Rust using `overlayfs`, cgroups, and Linux namespaces with a **double fork** architecture.

## Overview

**Rustbox** creates a secure and minimal sandbox environment on Linux. It uses:
- **OverlayFS** for isolated file systems
- **Cgroups v2** to restrict memory and CPU usage
- **Linux namespaces** to isolate the process (PID, UTS, IPC, NET, USER)
- **Double fork** architecture for proper process isolation and resource cleanup

This tool is useful for running untrusted code in a controlled environment, testing, or creating lightweight containers.

## Double Fork Implementation

RustBox employs a **double fork** pattern to ensure proper process isolation and clean resource management:

### Process Hierarchy

```
[Outer Parent Process]
    └─> fork() #1
        ├─> [Namespaced Parent Process]
        │   ├─> unshare() - Creates new namespaces
        │   └─> fork() #2
        │       ├─> [Inner Child Process]
        │       │   ├─> Mount /proc and /dev
        │       │   ├─> chroot() to merged overlay
        │       │   ├─> chdir() to working directory
        │       │   └─> execv() - Execute shell/binary
        │       └─> [Namespaced Parent] waits for inner child
        │           └─> Unmounts /proc and /dev inside namespace
        └─> [Outer Parent] waits for namespaced parent
            ├─> Unmounts overlay filesystem
            └─> Cleans up cgroups
```

### Why Double Fork?

1. **First Fork (Outer → Namespaced Parent)**:
   - Isolates the namespace creation from the main process
   - Allows the outer parent to maintain control over cgroups and overlay mounts
   - Ensures cleanup happens outside the namespace context

2. **Second Fork (Namespaced Parent → Inner Child)**:
   - Creates PID 1 inside the new PID namespace
   - Provides proper process tree isolation
   - Enables the namespaced parent to handle cleanup of namespace-specific resources

3. **Cleanup Benefits**:
   - **Inner Child**: Executes user code in complete isolation
   - **Namespaced Parent**: Unmounts `/proc` and `/dev` after child exits (inside namespace)
   - **Outer Parent**: Unmounts overlay and removes cgroups (outside namespace)
   - Ensures resources are cleaned up in the correct order and context

## 🧰 Features

- **Isolated file system** using `overlayfs` with automatic cleanup
- **Memory and CPU constraints** with `cgroups v2`
- **Full namespace isolation** (PID, UTS, NET, USER, IPC)
- **Double fork architecture** for robust process management and resource cleanup
- **Custom shell or binary execution** inside the sandbox
- **Automatic resource cleanup** on exit (mounts, cgroups)
- Written in Rust with `nix` crate for safe syscall wrappers

## 📦 Requirements

- Linux kernel 5.x or higher (with overlayfs and cgroups v2 support)
- Rust (1.70+ recommended)
- Root privileges (for mounting and namespace ops)

## 🔧 Configuration

The sandbox is configured via the `SandboxConfig` struct:

```rust
pub struct SandboxConfig {
    pub base_dir: String,     // Base directory for overlayfs (e.g., ./rootfs)
    pub memory_limit: String, // Memory limit, e.g., "100M", "1G"
    pub cpu_limit: String,    // CPU limit as fraction, e.g., "0.5" (50% of one core)
    pub shell_path: String,   // Path to the shell or binary to execute
    pub workdir: String,      // Working directory inside container (e.g., "/")
}
```

### Command Line Usage

```bash
# Run with default settings
sudo ./target/debug/rustbox

# Custom configuration
sudo ./target/debug/rustbox \
    --base-dir ./rootfs \
    --memory 256M \
    --cpu-limit 0.5 \
    --shell /bin/bash \
    --workdir /root
```


## root remote debug

please refer: [connecting-to-lldb-server-agent](https://github.com/vadimcn/codelldb/blob/master/MANUAL.md#connecting-to-lldb-server-agent)

```
sudo lldb-server platform --server --listen 127.0.0.1:12345 ./target/debug/rustbox
```