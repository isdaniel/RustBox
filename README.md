# RustBox

> A Docker-like container runtime written in Rust with daemon architecture, supporting multi-container orchestration, persistent state management, and comprehensive CLI commands.

## Overview

**RustBox** is a production-ready container runtime that provides Docker-like functionality using:
- **Daemon Architecture** with Unix domain socket communication
- **Multi-container Management** with persistent state
- **OverlayFS** for isolated container filesystems  
- **Cgroups v2** for resource limits (memory, CPU)
- **Linux namespaces** for complete process isolation
- **Comprehensive CLI** with run, stop, list, inspect, remove, logs, and attach commands

This tool is designed for container orchestration, testing environments, and secure code execution.

## Architecture

### Daemon-Client Model

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                                RustBox Architecture                          │
└──────────────────────────────────────────────────────────────────────────────┘

[rustbox CLI]                                 [rustboxd Daemon]
     │                                               │
     │  Unix Socket                                  │
     │  /tmp/rustbox-daemon.sock                     │
     │                                               │
     │  IPC Protocol (JSON messages)                 │
     │  ───────────────────────────────────────────▶ │
     │  Commands:                                   │
     │   • run                                      │
     │   • stop                                     │
     │   • list                                     │
     │   • inspect                                  │
     │   • remove                                   │
     │   • logs                                     │
     │   • attach                                   │
     │                                               ▼
     │                                    ┌────────────────────────────┐
     │                                    │  Container Manager         │
     │                                    │  ───────────────────────── │
     │                                    │  • Controls lifecycle      │
     │                                    │  • Creates sandbox env     │
     │                                    │  • Manages PTY + process   │
     │                                    └────────────────────────────┘
     │                                               │
     │                                               ▼
     │                                    ┌────────────────────────────┐
     │                                    │  Registry (HashMap<ID, Container>)│
     │                                    └────────────────────────────┘
     │                                               │
     │                     ┌────────────────────────────────────────────┐
     │                     │           Container Instances              │
     │                     └────────────────────────────────────────────┘
     │                         │              │              │
     │                         ▼              ▼              ▼
     │                    [Container 1]  [Container 2]  [Container N]
     │                         │              │              │
     │                  ┌──────────────────────────────────────────────┐
     │                  │              Sandbox Components               │
     │                  │  overlayfs + cgroups + namespaces             │
     │                  └──────────────────────────────────────────────┘
     │                         │
     │                         │
     │  (When attaching)       │
     │  ───────────────────────────────────────────────────────────────────────────
    ┌─────────────────────────────────────────────────────────────────────────────┐
    │                            Container Attach Flow                            │
    └─────────────────────────────────────────────────────────────────────────────┘

    Client (e.g. docker attach, CLI, web terminal)
    │
    │  1. Send/receive stdin/stdout over Unix socket
    ▼
    ┌───────────────────────────────────────────────────────────┐
    │ Daemon Process                                            │
    │ ───────────────────────────────────────────────────────── │
    │  • Manages container lifecycle                            │
    │  • Holds PTY master side                                  │
    │  • Forwards data between client and container             │
    │                                                           │
    │  ┌──────────────────────────────────────────────────────┐ │
    │  │ Unix Socket (Client ↔ Daemon)                        │ │
    │  │  - AttachStdin  (client → daemon)                    │ │
    │  │  - AttachStdout (daemon → client)                    │ │
    │  └──────────────────────────────────────────────────────┘ │
    │                              │
    │                              │ (I/O forwarding loop) 
    │                              ▼
    │  ┌──────────────────────────────────────────────────────┐ │
    │  │ PTY Master                                           │ │
    │  │  - Pseudo terminal device endpoint controlled by     │ │
    │  │    the daemon                                        │ │
    │  │  - Reads container output                            │ │
    │  │  - Writes client input                               │ │
    │  └──────────────────────────────────────────────────────┘ │
    │                              │
    │                              │ (kernel-level link)
    │                              ▼
    │  ┌──────────────────────────────────────────────────────┐ │
    │  │ PTY Slave                                            │ │
    │  │  - Exposed inside the container as /dev/tty or stdin │ │
    │  │  - Attached to the container’s process (e.g. /bin/bash)││
    │  │  - Container writes stdout/stderr → goes to Master   │ │
    │  │  - Container reads stdin ← comes from Master         │ │
    │  └──────────────────────────────────────────────────────┘ │
    └───────────────────────────────────────────────────────────┘
    │
    ▼
    Container Process (e.g. /bin/bash, sh)
    • Reads from stdin (/dev/tty)
    • Writes to stdout/stderr (/dev/tty)

    ───────────────────────────────────────────────────────────────
    Summary:
    - PTY Master: controlled by the daemon, mediates all I/O
    - PTY Slave : presented to the container process as its terminal
    - Unix Socket: transports attach stream between client ↔ daemon
    ───────────────────────────────────────────────────────────────

```

### Container Isolation

RustBox employs a **double fork** pattern for each container to ensure proper isolation:

### Process Hierarchy (Per Container)

```
[Daemon Process]
    └─> spawn_blocking()
        └─> [Container Task]
            └─> fork() #1
                ├─> [Namespaced Parent Process]
                │   ├─> unshare() - Creates new namespaces
                │   ├─> setup cgroups and overlay
                │   └─> fork() #2
                │       ├─> [Inner Child Process]
                │       │   ├─> Mount /proc and /dev
                │       │   ├─> chroot() to merged overlay
                │       │   ├─> chdir() to working directory
                │       │   └─> execv() - Execute command
                │       └─> [Namespaced Parent] waits for inner child
                │           └─> Unmounts /proc and /dev inside namespace
                └─> [Container Task] waits for namespaced parent
                    ├─> Unmounts overlay filesystem
                    ├─> Cleans up cgroups
                    └─> Updates container state in registry
```

### Container Lifecycle States

```
Created ──(start)──> Running ──(exit)──────> Exited
                       │
                       └──(stop)──> Stopped ──(exit)──> Exited
```

### Persistent State Management

- **Container metadata**: `/var/lib/rustbox/containers/<container-id>.json`
- **Container logs**: `/var/lib/rustbox/logs/<container-id>/`
- **Overlay filesystems**: `/var/lib/rustbox/overlay/<container-id>/`
- **State recovery**: Daemon recovers container state on restart

## Features

- **Daemon Architecture** with background process and client-server communication
- **Multi-container Management** supporting concurrent container execution
- **Persistent State Management** with automatic recovery across daemon restarts
- **Complete Container Lifecycle** (create, start, stop, remove, inspect)
- **Interactive Attach Support** with TTY allocation and real-time I/O streaming
- **Real-time Logging** with per-container stdout/stderr files
- **Resource Isolation** using cgroups v2 (memory, CPU limits)
- **Filesystem Isolation** using overlayfs with automatic cleanup
- **Full Namespace Isolation** (PID, UTS, NET, USER, IPC)
- **Docker-like CLI** with familiar commands (run, ps, logs, inspect, rm, attach)
- **Graceful Shutdown** with proper signal handling and resource cleanup
- **Security** with proper privilege separation and input validation

## Requirements

- Linux kernel 5.x or higher (with overlayfs and cgroups v2 support)
- Rust (1.70+ recommended) 
- Root privileges (for daemon operations, mounting, and namespace creation)

## Installation

### Build from Source

```bash
git clone https://github.com/isdaniel/RustBox.git
cd RustBox
cargo build --release
```

### Binaries

After building, you'll have two binaries:
- `rustbox` - Client CLI tool
- `daemon_rs` - Background daemon process

## Usage

### Start the Daemon

```bash
# Start the daemon in background (requires root)
sudo ./target/release/daemon_rs 2>&1 &
```

The daemon will:
- Listen on Unix socket `/tmp/rustbox-daemon.sock`
- Create system directories under `/var/lib/rustbox/`
- Recover existing container state from disk
- Handle graceful shutdown on SIGTERM/SIGINT

### Container Management

#### Create and Run Containers

```bash
# Run a container in background with TTY support (allows interactive attach)
sudo ./target/release/rustbox run --tty --memory 256M --cpu 0.5 /bin/bash

# Run a container with custom name
sudo ./target/release/rustbox run --tty --memory 256M --cpu 0.5 /bin/bash 2>&1

# Run a non-interactive container
sudo ./target/release/rustbox run --memory 256M /usr/bin/python3 script.py
```

**Note**: The `--tty` flag is required if you want to attach to the container later.

#### List Containers

```bash
# List running containers
sudo ./target/release/rustbox list

# List all containers (including stopped)
sudo ./target/release/rustbox list -a
# or
sudo ./target/release/rustbox list --all
```

#### Attach to Running Containers

```bash
# Attach to a running container (container must have been created with --tty flag)
sudo ./target/release/rustbox attach <container-id>

# Example:
sudo ./target/release/rustbox attach f1a5f84880a1
```

**Interactive Controls**:
- Press `Ctrl+P` followed by `Ctrl+Q` to detach from container (leaves it running)
- Press `Ctrl+C` to send interrupt signal and exit

**Requirements**:
- Container must have been started with `--tty` flag
- Container must be in `Running` state

#### Stop, Remove, and Inspect Containers

```bash
# Stop a running container
sudo ./target/release/rustbox stop <container-id>

# View container logs
sudo ./target/release/rustbox logs <container-id>
sudo ./target/release/rustbox logs --tail 50 <container-id>

# Inspect container details
sudo ./target/release/rustbox inspect <container-id>

# Remove a stopped container
sudo ./target/release/rustbox remove <container-id>

# Force remove a running container
sudo ./target/release/rustbox remove --force <container-id>
```

**Available Options**:
- `--name` - Custom container name (auto-generated if not provided)
- `--memory` - Memory limit (e.g., "256M", "1G", "512000")
- `--cpu` - CPU limit as fraction of one core (e.g., "0.5", "1.0")
- `--workdir` - Working directory inside container (default: "/")
- `--rootfs` - Path to rootfs directory (default: "./rootfs")
- `--tty` - Allocate a pseudo-TTY for interactive use (required for attach)

## Directory Structure

### Runtime Directories (created by daemon)

```
/var/lib/rustbox/
├── containers/           # Container metadata (JSON files)
│   ├── a1b2c3d4e5f6.json
│   └── f6e5d4c3b2a1.json
├── logs/                 # Container logs
│   ├── a1b2c3d4e5f6/
│   │   ├── stdout.log
│   │   └── stderr.log
│   └── f6e5d4c3b2a1/
│       ├── stdout.log
│       └── stderr.log
└── overlay/              # Overlay filesystem layers
    ├── a1b2c3d4e5f6/
    │   ├── lowerdir/     # Read-only base layer
    │   ├── upperdir/     # Container changes
    │   ├── workdir/      # Overlay work directory
    │   └── merged/       # Final mounted filesystem
    └── f6e5d4c3b2a1/
        ├── lowerdir/
        ├── upperdir/
        ├── workdir/
        └── merged/
```

### Source Code Structure

```
src/
├── lib.rs                # Public API exports
├── main.rs               # Client CLI entry point
├── daemon/               # Daemon implementation
│   ├── main.rs          # Daemon entry point
│   ├── server.rs        # Unix socket server
│   ├── container_manager.rs # Container lifecycle management
│   └── signal_handler.rs # Graceful shutdown handling
├── ipc/                  # Inter-process communication
│   ├── protocol.rs      # Message types and framing
│   └── client.rs        # Client-side socket communication
├── container/            # Container abstractions
│   ├── mod.rs           # Container data structures
│   ├── config.rs        # Configuration and validation
│   ├── sandbox.rs       # Core isolation logic
│   ├── state_machine.rs # Container state transitions
│   └── id.rs            # ID generation and validation
├── storage/              # Persistent storage
│   ├── metadata.rs      # Container metadata management
│   └── logs.rs          # Log file management  
├── cli/                  # CLI command implementations
│   ├── run.rs           # Create and start containers
│   ├── stop.rs          # Stop containers
│   ├── list.rs          # List containers
│   ├── inspect.rs       # Container details
│   ├── remove.rs        # Remove containers
│   ├── logs.rs          # View container logs
│   └── attach.rs        # Attach to containers
└── error.rs             # Error handling
```

## Technical Details

### IPC Protocol

Communication between client and daemon uses length-prefixed JSON messages over Unix domain sockets:

```
[4-byte length (u32, big-endian)][JSON payload]
```

Example:
```
0x0000001E  {"type":"ListRequest","all":true}
```

### Container ID Format

- 12-character hexadecimal identifiers (e.g., `a1b2c3d4e5f6`)
- Auto-generated names follow `adjective-noun` pattern (e.g., `happy-elephant`)
- CLI commands accept either ID or name

### Resource Limits

- **Memory**: Supports units like `100M`, `1G`, `512000` (bytes)
- **CPU**: Fraction of one core, e.g., `0.5` for 50% CPU limit
- Enforced via cgroups v2 at `/sys/fs/cgroup/rustbox/<container-id>/`

### Security Model

- Daemon runs as root for privileged operations
- Client commands run as user, connect via Unix socket
- Containers run in isolated namespaces (PID, NET, UTS, IPC, USER)
- Input validation prevents directory traversal and injection attacks

