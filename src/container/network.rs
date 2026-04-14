use serde::{Deserialize, Serialize};
use std::fs;
use std::net::Ipv4Addr;
use std::path::Path;
use std::process::Command;
use tracing::{info, warn};

pub const BRIDGE_NAME: &str = "rustbox0";
pub const BRIDGE_IP: &str = "10.88.0.1";
pub const BRIDGE_CIDR: &str = "10.88.0.1/16";
pub const SUBNET: &str = "10.88.0.0/16";
pub const SUBNET_PREFIX_LEN: u8 = 16;
const IP_COUNTER_PATH: &str = "/tmp/rustbox/network/ip_counter";

/// Port mapping configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub host_port: u16,
    pub container_port: u16,
    pub protocol: String,
}

/// Network configuration for a container
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    pub container_ip: Ipv4Addr,
    pub gateway_ip: Ipv4Addr,
    pub veth_host: String,
    pub veth_container: String,
    pub port_mappings: Vec<PortMapping>,
}

/// Parse port mapping specifications like "8080:80" or "8080:80/tcp"
pub fn parse_port_mappings(specs: &[String]) -> Vec<PortMapping> {
    specs
        .iter()
        .filter_map(|spec| {
            let (port_part, protocol) = if let Some((ports, proto)) = spec.rsplit_once('/') {
                (ports, proto.to_string())
            } else {
                (spec.as_str(), "tcp".to_string())
            };

            let (host_port_str, container_port_str) = port_part.split_once(':')?;
            let host_port = host_port_str.parse().ok()?;
            let container_port = container_port_str.parse().ok()?;

            Some(PortMapping {
                host_port,
                container_port,
                protocol,
            })
        })
        .collect()
}

/// Run a command and return stdout on success, or an error message on failure.
fn run_cmd(cmd: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to execute {cmd}: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "{cmd} {} failed (exit {}): {}",
            args.join(" "),
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ))
    }
}

/// Check if a command succeeds (exit code 0).
fn cmd_ok(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Ensure the rustbox0 bridge exists and is configured.
pub fn ensure_bridge() -> Result<(), String> {
    // Check if bridge already exists
    if cmd_ok("ip", &["link", "show", BRIDGE_NAME]) {
        info!("Bridge {} already exists", BRIDGE_NAME);
        return Ok(());
    }

    info!("Creating bridge {}", BRIDGE_NAME);
    run_cmd("ip", &["link", "add", "name", BRIDGE_NAME, "type", "bridge"])?;
    run_cmd("ip", &["addr", "add", BRIDGE_CIDR, "dev", BRIDGE_NAME])?;
    run_cmd("ip", &["link", "set", BRIDGE_NAME, "up"])?;

    info!("Bridge {} created with IP {}", BRIDGE_NAME, BRIDGE_CIDR);
    Ok(())
}

/// Enable IP forwarding and set up NAT masquerade rules.
pub fn setup_nat() -> Result<(), String> {
    // Enable IP forwarding
    fs::write("/proc/sys/net/ipv4/ip_forward", "1")
        .map_err(|e| format!("Failed to enable IP forwarding: {e}"))?;

    // Check if MASQUERADE rule already exists
    let nat_rules = run_cmd("iptables", &["-t", "nat", "-S", "POSTROUTING"]).unwrap_or_default();
    if !nat_rules.contains(SUBNET) {
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-A", "POSTROUTING", "-s", SUBNET, "!", "-o", BRIDGE_NAME, "-j",
                "MASQUERADE",
            ],
        )?;
        info!("Added NAT MASQUERADE rule for {}", SUBNET);
    }

    // Check if FORWARD rules already exist
    let fwd_rules = run_cmd("iptables", &["-S", "FORWARD"]).unwrap_or_default();
    if !fwd_rules.contains(&format!("-i {BRIDGE_NAME} -j ACCEPT")) {
        run_cmd(
            "iptables",
            &["-A", "FORWARD", "-i", BRIDGE_NAME, "-j", "ACCEPT"],
        )?;
    }
    if !fwd_rules.contains(&format!("-o {BRIDGE_NAME}")) {
        run_cmd(
            "iptables",
            &[
                "-A", "FORWARD", "-o", BRIDGE_NAME, "-m", "conntrack", "--ctstate",
                "RELATED,ESTABLISHED", "-j", "ACCEPT",
            ],
        )?;
    }

    info!("NAT rules configured");
    Ok(())
}

/// Allocate a unique IP address for a container.
pub fn allocate_ip() -> Result<Ipv4Addr, String> {
    let counter_dir = Path::new(IP_COUNTER_PATH).parent().unwrap();
    fs::create_dir_all(counter_dir)
        .map_err(|e| format!("Failed to create network state dir: {e}"))?;

    let counter: u32 = fs::read_to_string(IP_COUNTER_PATH)
        .unwrap_or_else(|_| "2".to_string())
        .trim()
        .parse()
        .unwrap_or(2);

    // Next IP: 10.88.{high}.{low} where counter = high*256 + low
    // Skip .0 (network) and .1 (gateway)
    let next = if counter < 2 { 2 } else { counter };
    let high = (next >> 8) as u8;
    let low = (next & 0xFF) as u8;

    // Save next counter
    fs::write(IP_COUNTER_PATH, (next + 1).to_string())
        .map_err(|e| format!("Failed to update IP counter: {e}"))?;

    let ip = Ipv4Addr::new(10, 88, high, low);
    info!("Allocated IP {} for container", ip);
    Ok(ip)
}

/// Create a veth pair and configure networking for a container.
///
/// The container must have been started with CLONE_NEWNET.
/// `container_pid` is the PID of the outer forked process (the namespaced parent).
pub fn create_veth_pair(
    container_id: &str,
    container_pid: i32,
    container_ip: Ipv4Addr,
) -> Result<NetworkConfig, String> {
    let short_id = &container_id[..6.min(container_id.len())];
    let veth_host = format!("veth_{short_id}");
    let veth_peer = format!("peer_{short_id}");
    let veth_container = "eth0".to_string();
    let pid_str = container_pid.to_string();
    let ip_cidr = format!("{}/{}", container_ip, SUBNET_PREFIX_LEN);

    info!(
        "Setting up veth pair for container {} (PID {}): {} <-> {}",
        container_id, container_pid, veth_host, veth_container
    );

    // Create veth pair with temporary peer name to avoid conflicts with host interfaces
    run_cmd(
        "ip",
        &[
            "link", "add", &veth_host, "type", "veth", "peer", "name", &veth_peer,
        ],
    )?;

    // Move container end into the container's network namespace
    run_cmd(
        "ip",
        &["link", "set", &veth_peer, "netns", &pid_str],
    )?;

    // Attach host end to bridge
    run_cmd(
        "ip",
        &["link", "set", &veth_host, "master", BRIDGE_NAME],
    )?;
    run_cmd("ip", &["link", "set", &veth_host, "up"])?;

    // Configure the container end via nsenter
    let netns_arg = format!("--net=/proc/{pid_str}/ns/net");

    // First rename the peer interface to eth0 inside the container namespace
    run_cmd(
        "nsenter",
        &[
            &netns_arg,
            "ip", "link", "set", &veth_peer, "name", &veth_container,
        ],
    )?;
    run_cmd(
        "nsenter",
        &[
            &netns_arg,
            "ip", "addr", "add", &ip_cidr, "dev", &veth_container,
        ],
    )?;
    run_cmd(
        "nsenter",
        &[
            &netns_arg,
            "ip", "link", "set", &veth_container, "up",
        ],
    )?;
    run_cmd(
        "nsenter",
        &[
            &netns_arg,
            "ip", "link", "set", "lo", "up",
        ],
    )?;
    run_cmd(
        "nsenter",
        &[
            &netns_arg,
            "ip", "route", "add", "default", "via", BRIDGE_IP,
        ],
    )?;

    info!(
        "Network configured for container {}: IP={}, gateway={}",
        container_id, container_ip, BRIDGE_IP
    );

    Ok(NetworkConfig {
        container_ip,
        gateway_ip: Ipv4Addr::new(10, 88, 0, 1),
        veth_host,
        veth_container,
        port_mappings: vec![],
    })
}

/// Set up iptables DNAT rules for port forwarding.
pub fn setup_port_forwarding(
    mappings: &[PortMapping],
    container_ip: Ipv4Addr,
) -> Result<(), String> {
    for mapping in mappings {
        let dest = format!("{}:{}", container_ip, mapping.container_port);
        let dport = mapping.host_port.to_string();
        let proto = &mapping.protocol;

        info!(
            "Setting up port forwarding: {} {} -> {}",
            proto, mapping.host_port, dest
        );

        // PREROUTING rule for external traffic
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-A", "PREROUTING", "-p", proto, "--dport", &dport, "-j", "DNAT",
                "--to-destination", &dest,
            ],
        )?;

        // OUTPUT rule for local traffic (from host to container via localhost)
        run_cmd(
            "iptables",
            &[
                "-t", "nat", "-A", "OUTPUT", "-p", proto, "--dport", &dport, "-j", "DNAT",
                "--to-destination", &dest,
            ],
        )?;
    }

    Ok(())
}

/// Clean up networking resources for a container.
pub fn cleanup_networking(config: &NetworkConfig) -> Result<(), String> {
    info!(
        "Cleaning up networking for container (IP={})",
        config.container_ip
    );

    // Remove port forwarding rules
    for mapping in &config.port_mappings {
        let dest = format!("{}:{}", config.container_ip, mapping.container_port);
        let dport = mapping.host_port.to_string();
        let proto = &mapping.protocol;

        // Remove PREROUTING rule (ignore errors - rule may not exist)
        if let Err(e) = run_cmd(
            "iptables",
            &[
                "-t", "nat", "-D", "PREROUTING", "-p", proto, "--dport", &dport, "-j", "DNAT",
                "--to-destination", &dest,
            ],
        ) {
            warn!("Failed to remove PREROUTING rule: {}", e);
        }

        // Remove OUTPUT rule
        if let Err(e) = run_cmd(
            "iptables",
            &[
                "-t", "nat", "-D", "OUTPUT", "-p", proto, "--dport", &dport, "-j", "DNAT",
                "--to-destination", &dest,
            ],
        ) {
            warn!("Failed to remove OUTPUT rule: {}", e);
        }
    }

    // Remove veth pair (removing host end also removes container end)
    if cmd_ok("ip", &["link", "show", &config.veth_host]) {
        if let Err(e) = run_cmd("ip", &["link", "del", &config.veth_host]) {
            warn!("Failed to remove veth {}: {}", config.veth_host, e);
        }
    }

    info!("Network cleanup complete");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_port_mappings_basic() {
        let specs = vec!["8080:80".to_string()];
        let mappings = parse_port_mappings(&specs);
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].host_port, 8080);
        assert_eq!(mappings[0].container_port, 80);
        assert_eq!(mappings[0].protocol, "tcp");
    }

    #[test]
    fn test_parse_port_mappings_with_protocol() {
        let specs = vec!["53:53/udp".to_string(), "8080:80/tcp".to_string()];
        let mappings = parse_port_mappings(&specs);
        assert_eq!(mappings.len(), 2);
        assert_eq!(mappings[0].host_port, 53);
        assert_eq!(mappings[0].container_port, 53);
        assert_eq!(mappings[0].protocol, "udp");
        assert_eq!(mappings[1].protocol, "tcp");
    }

    #[test]
    fn test_parse_port_mappings_invalid() {
        let specs = vec!["invalid".to_string(), "abc:def".to_string()];
        let mappings = parse_port_mappings(&specs);
        assert_eq!(mappings.len(), 0);
    }

    #[test]
    fn test_parse_port_mappings_empty() {
        let specs: Vec<String> = vec![];
        let mappings = parse_port_mappings(&specs);
        assert_eq!(mappings.len(), 0);
    }
}
