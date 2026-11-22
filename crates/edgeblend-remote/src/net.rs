//! LAN addresses for the magic link.

use std::net::{IpAddr, Ipv4Addr};

/// Enumerate local IPv4 addresses; prefer private ranges on non-virtual adapters.
pub fn lan_ips() -> Vec<Ipv4Addr> {
    let Ok(ifaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut preferred = Vec::new();
    let mut other = Vec::new();
    for iface in ifaces {
        if iface.is_loopback() {
            continue;
        }
        if is_virtual_adapter(&iface.name) {
            continue;
        }
        let IpAddr::V4(v4) = iface.ip() else {
            continue;
        };
        if v4.is_unspecified() || v4.is_multicast() || v4.is_link_local() {
            continue;
        }
        if is_private_v4(v4) {
            preferred.push(v4);
        } else {
            other.push(v4);
        }
    }
    preferred.sort_by_key(|ip| private_rank(*ip));
    preferred.append(&mut other);
    preferred.dedup();
    preferred
}

fn is_virtual_adapter(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const NEEDLES: &[&str] = &[
        "vmware",
        "virtualbox",
        "vbox",
        "hyper-v",
        "vethernet",
        "docker",
        "wsl",
        "vpn",
        "tap-windows",
        "loopback",
        "pseudo",
        "bluetooth",
    ];
    NEEDLES.iter().any(|s| n.contains(s))
}

fn is_private_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    matches!(o[0], 10)
        || (o[0] == 172 && (16..=31).contains(&o[1]))
        || (o[0] == 192 && o[1] == 168)
}

fn private_rank(v4: Ipv4Addr) -> u8 {
    let o = v4.octets();
    if o[0] == 192 && o[1] == 168 {
        0
    } else if o[0] == 10 {
        1
    } else if o[0] == 172 && (16..=31).contains(&o[1]) {
        2
    } else {
        3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_rank_prefers_192_168() {
        assert!(private_rank(Ipv4Addr::new(192, 168, 1, 10)) < private_rank(Ipv4Addr::new(10, 0, 0, 1)));
        assert!(is_private_v4(Ipv4Addr::new(172, 16, 0, 1)));
        assert!(!is_private_v4(Ipv4Addr::new(172, 32, 0, 1)));
        assert!(!is_private_v4(Ipv4Addr::new(8, 8, 8, 8)));
        assert!(is_virtual_adapter("vEthernet (WSL)"));
        assert!(!is_virtual_adapter("Ethernet"));
        assert!(!is_virtual_adapter("Wi-Fi"));
    }

    #[test]
    fn lan_ips_skip_loopback() {
        assert!(!lan_ips().iter().any(|ip| ip.is_loopback()));
    }
}
