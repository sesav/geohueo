use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressFamily {
    Any,
    Ipv4,
    Ipv6,
}

impl AddressFamily {
    fn select(self, addresses: impl IntoIterator<Item = SocketAddr>) -> Option<IpAddr> {
        addresses
            .into_iter()
            .map(|address| address.ip())
            .find(|ip| match self {
                Self::Any => true,
                Self::Ipv4 => ip.is_ipv4(),
                Self::Ipv6 => ip.is_ipv6(),
            })
    }
}

pub enum Target {
    Address(IpAddr),
    Hostname { host: String, ip: IpAddr },
}

impl Target {
    pub fn ip(&self) -> IpAddr {
        match self {
            Self::Address(ip) => *ip,
            Self::Hostname { ip, .. } => *ip,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Address(ip) => ip.to_string(),
            Self::Hostname { host, ip } => format!("{host} ({ip})"),
        }
    }
}

pub fn resolve(input: &str, family: AddressFamily) -> Result<Target, String> {
    // Family selection only applies to DNS results, not explicit IP addresses.
    if let Ok(ip) = input.parse::<IpAddr>() {
        return Ok(Target::Address(ip));
    }

    if input.is_empty()
        || input.chars().any(|c| {
            c.is_whitespace()
                || c.is_control()
                || matches!(c, ':' | '/' | '\\' | '[' | ']' | '@' | '?' | '#')
        })
    {
        return Err(format!(
            "cannot resolve '{input}': expected an IP address or hostname"
        ));
    }

    let addresses = (input, 0)
        .to_socket_addrs()
        .map_err(|error| format!("cannot resolve '{input}': {error}"))?;

    match family.select(addresses) {
        Some(ip) => Ok(Target::Hostname {
            host: input.to_string(),
            ip,
        }),
        None => {
            let reason = match family {
                AddressFamily::Any => "no addresses found",
                AddressFamily::Ipv4 => "no IPv4 addresses found",
                AddressFamily::Ipv6 => "no IPv6 addresses found",
            };
            Err(format!("cannot resolve '{input}': {reason}"))
        }
    }
}

pub fn no_data_message(ip: Option<IpAddr>) -> String {
    match ip.and_then(no_data_reason) {
        Some(reason) => format!("no geolocation data ({reason})"),
        None => "no geolocation data".to_string(),
    }
}

fn no_data_reason(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(ip) => {
            if ip.is_loopback() {
                Some("loopback address")
            } else if ip.is_private() {
                Some("private address")
            } else if ip.is_link_local() {
                Some("link-local address")
            } else if ip.is_broadcast() {
                Some("broadcast address")
            } else if ip.is_multicast() {
                Some("multicast address")
            } else if ip.is_unspecified() {
                Some("unspecified address")
            } else {
                None
            }
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return no_data_reason(IpAddr::V4(ip));
            }

            if ip.is_loopback() {
                Some("loopback address")
            } else if ip.is_unique_local() {
                Some("private address")
            } else if ip.is_unicast_link_local() {
                Some("link-local address")
            } else if ip.is_multicast() {
                Some("multicast address")
            } else if ip.is_unspecified() {
                Some("unspecified address")
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_ip_literals_without_dns() {
        for input in ["8.8.8.8", "2001:4860:4860::8888"] {
            for family in [AddressFamily::Any, AddressFamily::Ipv4, AddressFamily::Ipv6] {
                let target = resolve(input, family).unwrap();
                assert!(matches!(target, Target::Address(_)));
                assert_eq!(target.ip(), input.parse::<IpAddr>().unwrap());
                assert_eq!(target.label(), input);
            }
        }
    }

    #[test]
    fn selects_first_address_of_requested_family() {
        let ipv6: SocketAddr = "[2001:db8::1]:0".parse().unwrap();
        let ipv4: SocketAddr = "192.0.2.1:0".parse().unwrap();
        let next_ipv4: SocketAddr = "192.0.2.2:0".parse().unwrap();
        let addresses = [ipv6, ipv4, next_ipv4];

        assert_eq!(AddressFamily::Any.select(addresses), Some(ipv6.ip()));
        assert_eq!(AddressFamily::Ipv4.select(addresses), Some(ipv4.ip()));
        assert_eq!(AddressFamily::Ipv6.select(addresses), Some(ipv6.ip()));
        assert_eq!(AddressFamily::Any.select([ipv4, ipv6]), Some(ipv4.ip()));
    }

    #[test]
    fn does_not_fall_back_to_another_family() {
        let ipv6: SocketAddr = "[2001:db8::1]:0".parse().unwrap();
        let ipv4: SocketAddr = "192.0.2.1:0".parse().unwrap();

        assert_eq!(AddressFamily::Ipv4.select([ipv6]), None);
        assert_eq!(AddressFamily::Ipv6.select([ipv4]), None);
        assert_eq!(AddressFamily::Any.select([]), None);
    }

    #[test]
    fn rejects_malformed_targets_before_dns() {
        for input in [
            "",
            "[invalid]",
            "https://example.com",
            "example.com:443",
            "not a hostname",
            "example.com\0",
        ] {
            assert!(resolve(input, AddressFamily::Any).is_err());
        }
    }

    #[test]
    fn identifies_non_public_addresses() {
        for (input, reason) in [
            ("192.168.1.1", "private address"),
            ("127.0.0.1", "loopback address"),
            ("169.254.1.1", "link-local address"),
            ("255.255.255.255", "broadcast address"),
            ("224.0.0.1", "multicast address"),
            ("0.0.0.0", "unspecified address"),
            ("::1", "loopback address"),
            ("fc00::1", "private address"),
            ("fe80::1", "link-local address"),
            ("ff02::1", "multicast address"),
            ("::", "unspecified address"),
            ("::ffff:192.168.1.1", "private address"),
        ] {
            assert_eq!(no_data_reason(input.parse().unwrap()), Some(reason));
        }

        assert_eq!(no_data_reason("8.8.8.8".parse().unwrap()), None);
        assert_eq!(
            no_data_reason("2001:4860:4860::8888".parse().unwrap()),
            None
        );
        assert_eq!(no_data_message(None), "no geolocation data");
    }

    #[test]
    fn labels_resolved_hostname() {
        let target = Target::Hostname {
            host: "example.com".to_string(),
            ip: "192.0.2.1".parse().unwrap(),
        };

        assert_eq!(target.label(), "example.com (192.0.2.1)");
    }
}
