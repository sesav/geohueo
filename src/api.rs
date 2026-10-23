use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const API_BASE: &str = "https://json.geoiplookup.io";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Response returned by the geoiplookup.io API.
///
/// All fields have defaults so that the utility keeps working if the service
/// adds or removes fields from its response.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct IpInfo {
    pub ip: String,
    pub isp: String,
    pub org: String,
    pub hostname: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub postal_code: String,
    pub city: String,
    pub country_code: String,
    pub country_name: String,
    pub continent_code: String,
    pub continent_name: String,
    pub region: String,
    pub district: String,
    pub timezone_name: String,
    pub connection_type: String,
    pub asn_number: u64,
    pub asn_org: String,
    pub asn: String,
    pub currency_code: String,
    pub currency_name: String,
    pub language_code: String,
    pub language_name: String,
    pub success: bool,
    pub premium: bool,
}

pub struct Client {
    agent: ureq::Agent,
}

impl Client {
    pub fn new(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(concat!("geolkp/", env!("CARGO_PKG_VERSION")))
            .build();

        Self {
            agent: config.into(),
        }
    }

    /// Look up an IP address. `None` asks the API to look up the caller's address.
    pub fn lookup(&self, ip: Option<IpAddr>) -> Result<IpInfo> {
        let url = match ip {
            Some(ip) => format!("{API_BASE}/{ip}"),
            None => format!("{API_BASE}/"),
        };

        let mut response = self
            .agent
            .get(&url)
            .call()
            .with_context(|| format!("request to {url} failed"))?;
        response
            .body_mut()
            .read_json()
            .context("could not read geolocation response")
    }
}

impl Default for Client {
    fn default() -> Self {
        Self::new(DEFAULT_TIMEOUT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configures_requested_http_timeout() {
        let timeout = Duration::from_secs(5);
        let client = Client::new(timeout);
        assert_eq!(client.agent.config().timeouts().global, Some(timeout));
        assert_eq!(
            Client::default().agent.config().timeouts().global,
            Some(DEFAULT_TIMEOUT)
        );
    }

    #[test]
    fn accepts_partial_response_and_unknown_fields() {
        let info: IpInfo = serde_json::from_str(
            r#"{"ip":"8.8.8.8","success":true,"new_field":"ignored"}"#,
        )
        .unwrap();

        assert_eq!(info.ip, "8.8.8.8");
        assert!(info.success);
        assert_eq!(info.latitude, None);
        assert_eq!(info.longitude, None);
    }

    #[test]
    fn distinguishes_missing_coordinates_from_zero() {
        let info: IpInfo = serde_json::from_str(r#"{"latitude":null,"longitude":0}"#).unwrap();
        assert_eq!(info.latitude, None);
        assert_eq!(info.longitude, Some(0.0));
    }

    #[test]
    fn missing_success_is_not_treated_as_success() {
        let info: IpInfo = serde_json::from_str("{}").unwrap();
        assert!(!info.success);
    }
}
