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
    base_url: String,
}

impl Client {
    pub fn new(timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .user_agent(concat!("geolkp/", env!("CARGO_PKG_VERSION")))
            .build();

        Self {
            agent: config.into(),
            base_url: API_BASE.to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(base_url: String, timeout: Duration) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            // Loopback tests must not depend on proxy environment variables.
            .proxy(None)
            .build();
        Self {
            agent: config.into(),
            base_url,
        }
    }

    /// Look up an IP address. `None` asks the API to look up the caller's address.
    pub fn lookup(&self, ip: Option<IpAddr>) -> Result<IpInfo> {
        let url = match ip {
            Some(ip) => format!("{}/{ip}", self.base_url),
            None => format!("{}/", self.base_url),
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
    use crate::test_support::{Response, TestServer, unavailable_client};

    #[test]
    fn looks_up_ip_using_local_server() {
        let server = TestServer::new(vec![Response::Http {
            status: 200,
            body: r#"{"ip":"8.8.8.8","success":true}"#,
        }]);
        let info = server
            .client()
            .lookup(Some("8.8.8.8".parse().unwrap()))
            .unwrap();

        assert!(info.success);
        assert_eq!(info.ip, "8.8.8.8");
        assert_eq!(server.requests(), ["GET /8.8.8.8 HTTP/1.1"]);
    }

    #[test]
    fn connection_refused_preserves_request_context_and_cause() {
        let error = unavailable_client().lookup(None).unwrap_err();

        assert!(error.to_string().starts_with("request to http://127.0.0.1:"));
        assert!(matches!(
            error.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::Io(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused
        ));
    }

    #[test]
    fn http_failures_preserve_status_and_request_context() {
        for status in [429, 500, 503] {
            let server = TestServer::new(vec![Response::Http {
                status,
                body: r#"{"error":"unavailable"}"#,
            }]);
            let error = server.client().lookup(None).unwrap_err();

            assert_eq!(
                error.to_string(),
                format!("request to {}/ failed", server.url())
            );
            assert!(matches!(
                error.downcast_ref::<ureq::Error>(),
                Some(ureq::Error::StatusCode(code)) if *code == status
            ));
            assert!(format!("{error:#}").contains(&status.to_string()));
        }
    }

    #[test]
    fn disconnected_server_returns_an_error_without_panicking() {
        let server = TestServer::new(vec![Response::Disconnect]);
        let error = server.client().lookup(None).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("request to {}/ failed", server.url())
        );
        assert!(matches!(
            error.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::Io(_))
        ));
    }

    #[test]
    fn malformed_json_preserves_response_context_and_cause() {
        let server = TestServer::new(vec![Response::Http {
            status: 200,
            body: "not JSON",
        }]);
        let error = server.client().lookup(None).unwrap_err();

        assert_eq!(error.to_string(), "could not read geolocation response");
        assert!(matches!(
            error.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::Json(error)) if error.is_syntax()
        ));
    }

    #[test]
    fn times_out_while_waiting_for_response_headers() {
        let server = TestServer::new(vec![Response::StallBeforeHeaders]);
        let error = server.client().lookup(None).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!("request to {}/ failed", server.url())
        );
        assert!(matches!(
            error.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::Timeout(ureq::Timeout::Global))
        ));
    }

    #[test]
    fn times_out_while_reading_response_body() {
        let server = TestServer::new(vec![Response::StallDuringBody]);
        let error = server.client().lookup(None).unwrap_err();

        assert_eq!(error.to_string(), "could not read geolocation response");
        assert!(matches!(
            error.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::Timeout(ureq::Timeout::Global))
        ));
    }

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
