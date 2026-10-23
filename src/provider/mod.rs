//! Provider-specific request and response handling.

use anyhow::Result;

use crate::model::{LookupResult, LookupTarget};

pub mod geoiplookup;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub ipv4: bool,
    pub ipv6: bool,
    pub self_ip: bool,
}

impl Capabilities {
    pub fn supports(self, target: LookupTarget) -> bool {
        match target {
            LookupTarget::SelfIp => self.self_ip,
            LookupTarget::Ip(ip) if ip.is_ipv4() => self.ipv4,
            LookupTarget::Ip(_) => self.ipv6,
        }
    }
}

/// An object-safe interface for looking up IP geolocation through a provider.
pub trait GeoProvider {
    fn capabilities(&self) -> Capabilities;
    fn lookup(&self, target: LookupTarget) -> Result<LookupResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capabilities_distinguish_self_ip_and_address_families() {
        let capabilities = Capabilities {
            ipv4: true,
            ipv6: false,
            self_ip: false,
        };
        assert!(capabilities.supports(LookupTarget::Ip("8.8.8.8".parse().unwrap())));
        assert!(!capabilities.supports(LookupTarget::Ip("2001:4860:4860::8888".parse().unwrap())));
        assert!(!capabilities.supports(LookupTarget::SelfIp));
    }
}
