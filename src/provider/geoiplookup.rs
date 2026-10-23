use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::error::{ErrorCode, LookupError};
use crate::model::{GeoRecord, LookupResult, LookupTarget};
use crate::provider::{Capabilities, GeoProvider};
use crate::transport::HttpTransport;

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
    transport: HttpTransport,
    base_url: String,
}

impl Client {
    pub fn new(timeout: Duration) -> Self {
        Self {
            transport: HttpTransport::new(timeout),
            base_url: API_BASE.to_string(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(base_url: String, timeout: Duration) -> Self {
        Self {
            transport: HttpTransport::for_test(timeout),
            base_url,
        }
    }

    /// Look up an IP address. `None` asks the API to look up the caller's address.
    fn lookup_response(&self, ip: Option<IpAddr>) -> Result<IpInfo> {
        let url = match ip {
            Some(ip) => format!("{}/{ip}", self.base_url),
            None => format!("{}/", self.base_url),
        };

        let mut response = self.transport.get(&url)?;
        response
            .body_mut()
            .read_json()
            .context("could not read geolocation response")
    }
}

impl GeoProvider for Client {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            ipv4: true,
            ipv6: true,
            self_ip: true,
        }
    }

    fn lookup(&self, target: LookupTarget) -> Result<LookupResult> {
        let ip = match target {
            LookupTarget::SelfIp => None,
            LookupTarget::Ip(ip) => Some(ip),
        };
        let info = self
            .lookup_response(ip)
            .map_err(|error| LookupError::from_error(&error, ErrorCode::Decode))?;
        let record = GeoRecord::from(&info);
        let legacy_json = serde_json::to_string(&info).context("could not serialize response")?;
        Ok(LookupResult::new(record, info.success).with_legacy_json(legacy_json))
    }
}

impl From<&IpInfo> for GeoRecord {
    fn from(info: &IpInfo) -> Self {
        fn present(value: &str) -> Option<String> {
            (!value.is_empty()).then(|| value.to_string())
        }

        Self {
            ip: present(&info.ip),
            isp: present(&info.isp),
            org: present(&info.org),
            hostname: present(&info.hostname),
            latitude: info.latitude,
            longitude: info.longitude,
            postal_code: present(&info.postal_code),
            city: present(&info.city),
            country_code: present(&info.country_code),
            country_name: present(&info.country_name),
            continent_code: present(&info.continent_code),
            continent_name: present(&info.continent_name),
            region: present(&info.region),
            district: present(&info.district),
            timezone_name: present(&info.timezone_name),
            connection_type: present(&info.connection_type),
            asn_number: (info.asn_number != 0).then_some(info.asn_number),
            asn_org: present(&info.asn_org),
            asn: present(&info.asn),
            currency_code: present(&info.currency_code),
            currency_name: present(&info.currency_name),
            language_code: present(&info.language_code),
            language_name: present(&info.language_name),
            ..GeoRecord::default()
        }
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
            .lookup_response(Some("8.8.8.8".parse().unwrap()))
            .unwrap();

        assert!(info.success);
        assert_eq!(info.ip, "8.8.8.8");
        assert_eq!(server.requests(), ["GET /8.8.8.8 HTTP/1.1"]);
    }

    #[test]
    fn provider_contract_normalizes_response_and_preserves_legacy_json() {
        let server = TestServer::new(vec![Response::Http {
            status: 200,
            body: r#"{"ip":"2001:4860:4860::8888","country_code":"US","latitude":0,"asn_number":15169,"success":true,"extra":"ignored"}"#,
        }]);
        let client = server.client();
        let provider: &dyn GeoProvider = &client;
        let result = provider
            .lookup(LookupTarget::Ip("2001:4860:4860::8888".parse().unwrap()))
            .unwrap();

        assert!(provider.capabilities().supports(LookupTarget::SelfIp));
        assert!(result.success);
        assert_eq!(result.record.ip.as_deref(), Some("2001:4860:4860::8888"));
        assert_eq!(result.record.country_code.as_deref(), Some("US"));
        assert_eq!(result.record.city, None);
        assert_eq!(result.record.latitude, Some(0.0));
        assert_eq!(result.record.asn_number, Some(15169));
        let legacy: IpInfo = serde_json::from_str(result.legacy_json().unwrap()).unwrap();
        assert_eq!(
            result.legacy_json().unwrap(),
            serde_json::to_string(&legacy).unwrap()
        );
        assert_eq!(server.requests(), ["GET /2001:4860:4860::8888 HTTP/1.1"]);
    }

    #[test]
    fn provider_uses_root_for_self_lookup_and_keeps_no_data_result() {
        let server = TestServer::new(vec![Response::Http {
            status: 200,
            body: r#"{"ip":"192.168.0.1","success":false}"#,
        }]);
        let result = GeoProvider::lookup(&server.client(), LookupTarget::SelfIp).unwrap();

        assert!(!result.success);
        assert_eq!(result.record.ip.as_deref(), Some("192.168.0.1"));
        assert_eq!(server.requests(), ["GET / HTTP/1.1"]);
    }

    #[test]
    fn connection_refused_preserves_request_context_and_cause() {
        let error = unavailable_client().lookup_response(None).unwrap_err();

        assert!(
            error
                .to_string()
                .starts_with("request to http://127.0.0.1:")
        );
        let cause = error.downcast_ref::<ureq::Error>();
        assert!(
            matches!(
                cause,
                Some(ureq::Error::Io(error)) if error.kind() == std::io::ErrorKind::ConnectionRefused
            ),
            "expected ConnectionRefused, got {cause:?}; full error: {error:#}"
        );
    }

    #[test]
    fn http_failures_preserve_status_and_request_context() {
        for status in [429, 500, 503] {
            let server = TestServer::new(vec![Response::Http {
                status,
                body: r#"{"error":"unavailable"}"#,
            }]);
            let error = server.client().lookup_response(None).unwrap_err();

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
        let error = server.client().lookup_response(None).unwrap_err();

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
        let error = server.client().lookup_response(None).unwrap_err();

        assert_eq!(error.to_string(), "could not read geolocation response");
        assert!(matches!(
            error.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::Json(error)) if error.is_syntax()
        ));
    }

    #[test]
    fn times_out_while_waiting_for_response_headers() {
        let server = TestServer::new(vec![Response::StallBeforeHeaders]);
        let error = server.client().lookup_response(None).unwrap_err();

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
        let error = server.client().lookup_response(None).unwrap_err();

        assert_eq!(error.to_string(), "could not read geolocation response");
        assert!(matches!(
            error.downcast_ref::<ureq::Error>(),
            Some(ureq::Error::Timeout(ureq::Timeout::Global))
        ));
    }

    #[test]
    fn accepts_partial_response_and_unknown_fields() {
        let info: IpInfo =
            serde_json::from_str(r#"{"ip":"8.8.8.8","success":true,"new_field":"ignored"}"#)
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
