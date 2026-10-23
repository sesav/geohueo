//! MaxMind GeoIP and GeoLite web services (not the local MMDB database).

use std::net::IpAddr;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::Value;

use crate::config::{MaxMindProduct, MaxMindService, MaxMindSettings};
use crate::error::{ErrorCode, LookupError};
use crate::model::{GeoRecord, GeoSubdivision, LookupResult, LookupTarget};
use crate::provider::{Capabilities, GeoProvider};
use crate::transport::HttpTransport;

const MAX_BODY: u64 = 256 * 1024;

pub(crate) struct Client {
    transport: HttpTransport,
    base_url: String,
    product: MaxMindProduct,
    locales: Vec<String>,
    authorization: String,
}

impl Client {
    pub fn new(settings: MaxMindSettings, timeout: Duration) -> Self {
        let base_url = match settings.service {
            MaxMindService::Geoip => "https://geoip.maxmind.com",
            MaxMindService::Geolite => "https://geolite.info",
        };
        Self::build(
            settings,
            HttpTransport::authorized(timeout),
            base_url.to_string(),
        )
    }

    fn build(settings: MaxMindSettings, transport: HttpTransport, base_url: String) -> Self {
        let authorization = format!(
            "Basic {}",
            STANDARD.encode(format!("{}:{}", settings.account_id, settings.license_key))
        );
        Self {
            transport,
            base_url,
            product: settings.product,
            locales: settings.locales,
            authorization,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(settings: MaxMindSettings, base_url: String) -> Self {
        Self::build(
            settings,
            HttpTransport::authorized_for_test(Duration::from_secs(1)),
            base_url,
        )
    }

    fn lookup_response(&self, target: LookupTarget) -> Result<GeoRecord> {
        let ip = match target {
            LookupTarget::SelfIp => "me".to_string(),
            LookupTarget::Ip(ip) => ip.to_string(),
        };
        let product = match self.product {
            MaxMindProduct::Country => "country",
            MaxMindProduct::City => "city",
            MaxMindProduct::Insights => "insights",
        };
        let url = format!("{}/geoip/v2.1/{product}/{ip}", self.base_url);
        let mut response = self.transport.get_authorized(&url, &self.authorization)?;
        let status = response.status().as_u16();
        if status != 200 {
            let code = if is_json(response.headers().get("content-type")) {
                response
                    .body_mut()
                    .with_config()
                    .limit(MAX_BODY)
                    .read_to_vec()
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .and_then(|json| json["code"].as_str().map(str::to_owned))
            } else {
                None
            };
            return Err(api_error(status, code.as_deref()));
        }
        if !is_json(response.headers().get("content-type")) {
            return Err(LookupError::new(ErrorCode::Decode, "MaxMind response is not JSON").into());
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_vec()
            .map_err(|error| {
                LookupError::from_http_client(&error, "could not read MaxMind response")
            })?;
        let json: Value = serde_json::from_slice(&bytes)
            .map_err(|_| LookupError::new(ErrorCode::Decode, "invalid MaxMind response JSON"))?;
        normalize(&json, &self.locales)
            .map_err(|error| LookupError::from_error(&error, ErrorCode::Mapping).into())
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
        Ok(LookupResult::new(self.lookup_response(target)?, true))
    }
}

fn is_json(content_type: Option<&ureq::http::HeaderValue>) -> bool {
    let Some(value) = content_type.and_then(|value| value.to_str().ok()) else {
        return false;
    };
    let mime = value
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    mime == "application/json" || (mime.starts_with("application/") && mime.ends_with("+json"))
}

fn api_error(status: u16, code: Option<&str>) -> anyhow::Error {
    let message = match code {
        Some("IP_ADDRESS_RESERVED") => "reserved or private IP address",
        Some("IP_ADDRESS_NOT_FOUND") => "no geolocation data",
        Some("IP_ADDRESS_INVALID" | "IP_ADDRESS_REQUIRED") => "invalid IP address",
        Some("AUTHORIZATION_INVALID" | "LICENSE_KEY_REQUIRED" | "ACCOUNT_ID_REQUIRED") => {
            "invalid MaxMind credentials"
        }
        Some("INSUFFICIENT_FUNDS") => "quota or service credit exhausted",
        Some("PERMISSION_REQUIRED" | "SERVICE_INVALID") => "service access denied",
        _ => match status {
            401 => "invalid MaxMind credentials",
            402 => "quota or service credit exhausted",
            403 => "service access denied",
            404 => "no geolocation data",
            429 => "rate limited",
            _ => "service request failed",
        },
    };
    let mut error = LookupError::http(status, format!("MaxMind HTTP {status}: {message}"));
    error.code = match code {
        Some("IP_ADDRESS_RESERVED" | "IP_ADDRESS_NOT_FOUND") => ErrorCode::NoData,
        Some("AUTHORIZATION_INVALID" | "LICENSE_KEY_REQUIRED" | "ACCOUNT_ID_REQUIRED") => {
            ErrorCode::Authentication
        }
        Some("INSUFFICIENT_FUNDS") => ErrorCode::QuotaExceeded,
        Some("PERMISSION_REQUIRED" | "SERVICE_INVALID") => ErrorCode::PermissionDenied,
        _ => error.code,
    };
    error.into()
}

fn normalize(json: &Value, locales: &[String]) -> Result<GeoRecord> {
    let ip = text(json, "/traits/ip_address")?
        .filter(|ip| ip.parse::<IpAddr>().is_ok())
        .ok_or_else(|| anyhow!("invalid MaxMind response: missing or invalid traits.ip_address"))?;
    let network = text(json, "/traits/network")?
        .filter(|network| !network.is_empty())
        .ok_or_else(|| anyhow!("invalid MaxMind response: missing traits.network"))?;
    let subdivisions = match json.pointer("/subdivisions") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                Ok(GeoSubdivision {
                    code: text(item, "/iso_code")?,
                    name: localized(item, "/names", locales)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
        _ => bail!("invalid MaxMind response at /subdivisions"),
    };

    Ok(GeoRecord {
        ip: Some(ip),
        network: Some(network),
        country_code: text(json, "/country/iso_code")?,
        country_name: localized(json, "/country/names", locales)?,
        registered_country_code: text(json, "/registered_country/iso_code")?,
        registered_country_name: localized(json, "/registered_country/names", locales)?,
        continent_code: text(json, "/continent/code")?,
        continent_name: localized(json, "/continent/names", locales)?,
        region: subdivisions.first().and_then(|part| part.name.clone()),
        subdivisions,
        city: localized(json, "/city/names", locales)?,
        postal_code: text(json, "/postal/code")?,
        latitude: decimal(json, "/location/latitude")?,
        longitude: decimal(json, "/location/longitude")?,
        accuracy_radius_km: integer(json, "/location/accuracy_radius")?
            .map(u32::try_from)
            .transpose()
            .context("invalid MaxMind response at /location/accuracy_radius")?,
        timezone_name: text(json, "/location/time_zone")?,
        asn_number: integer(json, "/traits/autonomous_system_number")?,
        asn_org: text(json, "/traits/autonomous_system_organization")?,
        isp: text(json, "/traits/isp")?,
        org: text(json, "/traits/organization")?,
        connection_type: text(json, "/traits/connection_type")?,
        ..GeoRecord::default()
    })
}

fn text(json: &Value, pointer: &str) -> Result<Option<String>> {
    match json.pointer(pointer) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        _ => bail!("invalid MaxMind response at {pointer}"),
    }
}

fn decimal(json: &Value, pointer: &str) -> Result<Option<f64>> {
    match json.pointer(pointer) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_f64()
            .map(Some)
            .ok_or_else(|| anyhow!("invalid MaxMind response at {pointer}")),
        _ => bail!("invalid MaxMind response at {pointer}"),
    }
}

fn integer(json: &Value, pointer: &str) -> Result<Option<u64>> {
    match json.pointer(pointer) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) if value.as_u64().is_some() => Ok(value.as_u64()),
        _ => bail!("invalid MaxMind response at {pointer}"),
    }
}

fn localized(json: &Value, pointer: &str, locales: &[String]) -> Result<Option<String>> {
    let Some(names) = json.pointer(pointer) else {
        return Ok(None);
    };
    if names.is_null() {
        return Ok(None);
    }
    let names = names
        .as_object()
        .ok_or_else(|| anyhow!("invalid MaxMind response at {pointer}"))?;
    for locale in locales
        .iter()
        .map(String::as_str)
        .chain(std::iter::once("en"))
    {
        match names.get(locale) {
            Some(Value::String(name)) => return Ok(Some(name.clone())),
            Some(Value::Null) | None => {}
            _ => bail!("invalid MaxMind response at {pointer}/{locale}"),
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Response, TestServer};

    fn settings(service: MaxMindService, product: MaxMindProduct) -> MaxMindSettings {
        MaxMindSettings {
            service,
            product,
            locales: vec!["ru".into(), "en".into()],
            account_id: "123456".into(),
            license_key: "test-license".into(),
        }
    }

    #[test]
    fn city_is_normalized_without_confusing_registered_country() {
        let server = TestServer::new(vec![Response::HttpWithType {
            status: 200,
            content_type: "application/vnd.maxmind.com-city+json; charset=UTF-8; version=2.1",
            body: r#"{"traits":{"ip_address":"8.8.8.8","network":"8.8.8.0/24","autonomous_system_number":15169},"country":{"iso_code":"US","names":{"en":"United States","ru":"США"}},"registered_country":{"iso_code":"CA"},"subdivisions":[{"iso_code":"CA","names":{"en":"California"}},{"iso_code":"LA"}],"city":{"names":{"en":"Mountain View"}},"location":{"latitude":0,"longitude":-122.1,"accuracy_radius":20}}"#,
        }]);
        let client = Client::for_test(
            settings(MaxMindService::Geoip, MaxMindProduct::City),
            server.url(),
        );
        let result = client
            .lookup(LookupTarget::Ip("8.8.8.8".parse().unwrap()))
            .unwrap();

        assert!(result.success);
        assert_eq!(result.record.country_name.as_deref(), Some("США"));
        assert_eq!(result.record.registered_country_code.as_deref(), Some("CA"));
        assert_eq!(result.record.region.as_deref(), Some("California"));
        assert_eq!(result.record.subdivisions.len(), 2);
        assert_eq!(result.record.latitude, Some(0.0));
        assert_eq!(result.record.accuracy_radius_km, Some(20));
        assert_eq!(result.record.asn_number, Some(15169));
        assert_eq!(server.requests(), ["GET /geoip/v2.1/city/8.8.8.8 HTTP/1.1"]);
        let headers = server.headers();
        let authorization = format!("Basic {}", STANDARD.encode("123456:test-license"));
        assert!(headers[0].iter().any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("Authorization") && value.trim() == authorization
            })
        }));
    }

    #[test]
    fn self_ip_and_ipv6_use_country_paths_without_optional_city_data() {
        let server = TestServer::new(vec![
            Response::Http {
                status: 200,
                body: r#"{"traits":{"ip_address":"8.8.8.8","network":"8.8.8.0/24"}}"#,
            },
            Response::Http {
                status: 200,
                body: r#"{"traits":{"ip_address":"2001:db8::1","network":"2001:db8::/64"}}"#,
            },
        ]);
        let client = Client::for_test(
            settings(MaxMindService::Geolite, MaxMindProduct::Country),
            server.url(),
        );
        assert!(client.lookup(LookupTarget::SelfIp).unwrap().success);
        let ipv6 = client
            .lookup(LookupTarget::Ip("2001:db8::1".parse().unwrap()))
            .unwrap();
        assert_eq!(ipv6.record.city, None);
        assert_eq!(
            server.requests(),
            [
                "GET /geoip/v2.1/country/me HTTP/1.1",
                "GET /geoip/v2.1/country/2001:db8::1 HTTP/1.1"
            ]
        );
    }

    #[test]
    fn insights_product_path_and_locale_fallback() {
        let server = TestServer::new(vec![Response::Http {
            status: 200,
            body: r#"{"traits":{"ip_address":"1.1.1.1","network":"1.1.1.0/24"},"country":{"names":{"en":"Australia"}}}"#,
        }]);
        let client = Client::for_test(
            settings(MaxMindService::Geoip, MaxMindProduct::Insights),
            server.url(),
        );
        let info = client
            .lookup(LookupTarget::Ip("1.1.1.1".parse().unwrap()))
            .unwrap();
        assert_eq!(info.record.country_name.as_deref(), Some("Australia"));
        assert_eq!(
            server.requests(),
            ["GET /geoip/v2.1/insights/1.1.1.1 HTTP/1.1"]
        );
    }

    #[test]
    fn errors_are_classified_without_exposing_server_text_or_secrets() {
        for (status, body, expected, code) in [
            (
                401,
                r#"{"code":"AUTHORIZATION_INVALID","error":"test-license"}"#,
                "credentials",
                ErrorCode::Authentication,
            ),
            (
                402,
                r#"{"code":"INSUFFICIENT_FUNDS","error":"test-license"}"#,
                "quota",
                ErrorCode::QuotaExceeded,
            ),
            (
                404,
                r#"{"code":"IP_ADDRESS_NOT_FOUND","error":"test-license"}"#,
                "no geolocation",
                ErrorCode::NoData,
            ),
            (
                400,
                r#"{"code":"IP_ADDRESS_RESERVED","error":"test-license"}"#,
                "reserved",
                ErrorCode::NoData,
            ),
            (403, "{}", "denied", ErrorCode::PermissionDenied),
            (429, "{}", "rate limited", ErrorCode::RateLimited),
            (503, "{}", "failed", ErrorCode::Http),
        ] {
            let server = TestServer::new(vec![Response::HttpWithType {
                status,
                body,
                content_type: "application/vnd.maxmind.com-error+json; charset=UTF-8; version=2.1",
            }]);
            let client = Client::for_test(
                settings(MaxMindService::Geoip, MaxMindProduct::City),
                server.url(),
            );
            let error = client.lookup(LookupTarget::SelfIp).unwrap_err();
            let typed = error.downcast_ref::<LookupError>().unwrap();
            assert_eq!(typed.code, code);
            assert_eq!(typed.http_status, Some(status));
            assert!(error.to_string().contains(expected));
            assert!(!format!("{error:#}").contains("test-license"));
        }
    }

    #[test]
    fn rejects_malformed_success_and_handles_non_json_error() {
        let server = TestServer::new(vec![Response::Http {
            status: 200,
            body: "{}",
        }]);
        let client = Client::for_test(
            settings(MaxMindService::Geoip, MaxMindProduct::Country),
            server.url(),
        );
        assert!(
            client
                .lookup(LookupTarget::SelfIp)
                .unwrap_err()
                .to_string()
                .contains("ip_address")
        );

        let server = TestServer::new(vec![Response::HttpWithType {
            status: 503,
            body: "maintenance",
            content_type: "text/plain",
        }]);
        let client = Client::for_test(
            settings(MaxMindService::Geoip, MaxMindProduct::City),
            server.url(),
        );
        assert!(
            client
                .lookup(LookupTarget::SelfIp)
                .unwrap_err()
                .to_string()
                .contains("503")
        );
    }

    #[test]
    fn authentication_is_not_forwarded_on_redirect() {
        let destination = TestServer::new(vec![Response::Http {
            status: 200,
            body: r#"{"traits":{"ip_address":"8.8.8.8","network":"8.8.8.0/24"}}"#,
        }]);
        let redirect = TestServer::new(vec![Response::Redirect {
            location: destination.url(),
        }]);
        let client = Client::for_test(
            settings(MaxMindService::Geoip, MaxMindProduct::City),
            redirect.url(),
        );
        let error = client.lookup(LookupTarget::SelfIp).unwrap_err();
        assert!(format!("{error:#}").contains("302") || format!("{error:#}").contains("redirect"));
        assert_eq!(redirect.requests(), ["GET /geoip/v2.1/city/me HTTP/1.1"]);
        assert!(destination.requests().is_empty());
    }
}
