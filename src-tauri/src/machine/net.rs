//! Network policy shared by the API layer and backends.

use std::net::IpAddr;

/// Is this an address a home embroidery machine could plausibly have —
/// i.e. a private (RFC 1918) or link-local IPv4 address?
///
/// This is also the security rule the localhost API applies to every target
/// address: the bridge refuses to act as a proxy to loopback services or the
/// public internet.
pub fn is_local_network_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local(),
        // Known machines are IPv4-only; refuse IPv6 targets for now.
        IpAddr::V6(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_network_rule() {
        assert!(is_local_network_ip("192.168.1.120".parse().unwrap()));
        assert!(is_local_network_ip("10.0.0.5".parse().unwrap()));
        assert!(is_local_network_ip("172.16.44.2".parse().unwrap()));
        assert!(is_local_network_ip("169.254.10.10".parse().unwrap()));
        assert!(!is_local_network_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_local_network_ip("127.0.0.1".parse().unwrap()));
        assert!(!is_local_network_ip("::1".parse().unwrap()));
        assert!(!is_local_network_ip("172.32.0.1".parse().unwrap()));
    }
}
