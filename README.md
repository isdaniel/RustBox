# Rustbox

> A lightweight sandboxing utility written in Rust using `overlayfs`, cgroups, and Linux namespaces.

## 🚀 Overview

**Rustbox** creates a secure and minimal sandbox environment on Linux. It uses:
- **OverlayFS** for isolated file systems
- **Cgroups** to restrict memory usage
- **Linux namespaces** to isolate the process (PID, UTS, IPC, NET, USER)

This tool is useful for running untrusted code in a controlled environment, testing, or creating lightweight containers.

## 🧰 Features

- Isolated file system using `overlayfs`
- Memory constraints with `cgroups v2`
- Full namespace isolation (PID, UTS, NET, USER, etc.)
- Custom shell or binary execution inside the sandbox
- Written purely in safe/unsafe Rust with `nix` and `std`

## 📦 Requirements

- Linux kernel 5.x or higher (with overlayfs and cgroups v2 support)
- Rust (1.70+ recommended)
- Root privileges (for mounting and namespace ops)

## 🔧 Configuration

The sandbox is configured via the `SandboxConfig` struct:

```rust
pub struct SandboxConfig {
    pub base_dir: String,     // Base directory for overlayfs (e.g., /tmp/sandbox)
    pub memory_limit: String, // Memory limit, e.g., "100M"
    pub shell_path: String,   // Path to the shell or binary to execute
}


## root remote debug

pleaes refer :　https://github.com/vadimcn/codelldb/blob/master/MANUAL.md#connecting-to-lldb-server-agent

```
sudo lldb-server platform --server --listen 127.0.0.1:12345 ./target/debug/rustbox
```