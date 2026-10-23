use anyhow::Result;

use geohueo::model::{GeoRecord, LookupResult, LookupTarget};
use geohueo::provider::{Capabilities, GeoProvider};

struct ExampleProvider;

impl GeoProvider for ExampleProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            ipv4: true,
            ipv6: false,
            self_ip: false,
        }
    }

    fn lookup(&self, target: LookupTarget) -> Result<LookupResult> {
        let LookupTarget::Ip(ip) = target else {
            anyhow::bail!("an IP address is required");
        };
        Ok(LookupResult::new(
            GeoRecord {
                ip: Some(ip.to_string()),
                country_code: Some("US".to_string()),
                ..GeoRecord::default()
            },
            true,
        ))
    }
}

#[test]
fn third_party_code_can_implement_the_provider_interface() {
    let provider: &dyn GeoProvider = &ExampleProvider;
    let target = LookupTarget::Ip("8.8.8.8".parse().unwrap());

    assert!(provider.capabilities().supports(target));
    assert!(!provider.capabilities().supports(LookupTarget::SelfIp));
    let result = provider.lookup(target).unwrap();
    assert!(result.success);
    assert_eq!(result.record.ip.as_deref(), Some("8.8.8.8"));
    assert_eq!(result.record.country_code.as_deref(), Some("US"));
}
