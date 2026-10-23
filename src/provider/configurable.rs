//! Declarative GET/JSON adapter. Templates only substitute a parsed IP address;
//! no arbitrary expressions, URL components, or response code are evaluated.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::config::{read_secret, validate_env_name};
use crate::error::{ErrorCode, LookupError};
use crate::model::{GeoRecord, LookupResult, LookupTarget};
use crate::provider::{Capabilities, GeoProvider};
use crate::transport::HttpTransport;

const MAX_BODY: u64 = 256 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RestProfile {
    pub timeout_seconds: Option<u64>,
    base_url: String,
    supports_ipv4: bool,
    supports_ipv6: bool,
    auth: Option<Auth>,
    request: Request,
    response: Response,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    method: Method,
    path: String,
    self_path: Option<String>,
    #[serde(default)]
    query: BTreeMap<String, String>,
    self_query: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Copy, Deserialize)]
enum Method {
    #[serde(rename = "GET")]
    Get,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    expected_status: Option<u16>,
    success_pointer: Option<String>,
    success_equals: Option<ExpectedValue>,
    error_message_pointer: Option<String>,
    #[serde(default)]
    required_fields: Vec<String>,
    #[serde(default)]
    fields: BTreeMap<String, String>,
    #[serde(default)]
    transforms: BTreeMap<String, Transform>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum ExpectedValue {
    Bool(bool),
    Integer(i64),
    String(String),
}

impl ExpectedValue {
    fn same_type(&self, actual: &Value) -> bool {
        match self {
            Self::Bool(_) => actual.is_boolean(),
            Self::Integer(_) => actual.as_i64().is_some(),
            Self::String(_) => actual.is_string(),
        }
    }

    fn matches(&self, actual: &Value) -> bool {
        match self {
            Self::Bool(expected) => actual.as_bool() == Some(*expected),
            Self::Integer(expected) => actual.as_i64() == Some(*expected),
            Self::String(expected) => actual.as_str() == Some(expected.as_str()),
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Transform {
    AsnToNumber,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Auth {
    None,
    Bearer {
        token_env: String,
    },
    Basic {
        username_env: String,
        password_env: String,
    },
    ApiKey {
        location: KeyLocation,
        name: String,
        value_env: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum KeyLocation {
    Header,
    Query,
}

enum Credential {
    None,
    Header { name: String, value: String },
    Query { name: String, value: String },
}

pub(crate) struct RestSettings {
    base_url: Url,
    capabilities: Capabilities,
    request: Request,
    response: Response,
    credential: Credential,
}

impl RestSettings {
    pub(crate) fn provider_label(&self) -> String {
        let host = self.base_url.host_str().unwrap_or("unknown");
        match self.base_url.port() {
            Some(port) => format!("rest-json {host}:{port}"),
            None => format!("rest-json {host}"),
        }
    }
}

impl RestProfile {
    pub fn validate(&self) -> Result<()> {
        let Method::Get = self.request.method;
        // Do not accept URL userinfo, paths, query strings, fragments, or parser
        // normalization that could smuggle an origin or a credential.
        if self.base_url.trim() != self.base_url
            || self.base_url.contains(['\\', '@'])
            || self.base_url.chars().any(char::is_control)
        {
            bail!("base_url must be an HTTPS origin without credentials");
        }
        let url = Url::parse(&self.base_url)
            .map_err(|_| anyhow!("base_url must be a valid HTTPS origin"))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("base_url must be an HTTPS origin without credentials, path, query or fragment");
        }
        let ip_in_path = validate_path(&self.request.path, true)?;
        let ip_in_query = validate_query(&self.request.query, true)?;
        if self.supports_ipv4 || self.supports_ipv6 {
            if usize::from(ip_in_path) + usize::from(ip_in_query) != 1 {
                bail!("request requires exactly one {{ip}} in a path segment or query value");
            }
        } else if ip_in_path || ip_in_query {
            bail!("IP template needs supports_ipv4 or supports_ipv6");
        }
        if let Some(path) = &self.request.self_path {
            validate_path(path, false)?;
            if let Some(query) = &self.request.self_query {
                validate_query(query, false)?;
            } else if ip_in_query {
                bail!("self_query is required when request.query contains {{ip}}");
            }
        } else if self.request.self_query.is_some() {
            bail!("self_query requires self_path");
        }
        if !self.supports_ipv4 && !self.supports_ipv6 && self.request.self_path.is_none() {
            bail!("profile supports no lookup targets");
        }

        self.response.validate()?;
        if let Some(auth) = &self.auth {
            auth.validate(&self.request)?;
        }
        Ok(())
    }

    pub fn resolve(&self) -> Result<RestSettings> {
        self.resolve_with(read_secret)
            .map_err(|error| LookupError::from_error(&error, ErrorCode::Credentials).into())
    }

    fn resolve_with(&self, read: impl Fn(&str) -> Result<String>) -> Result<RestSettings> {
        let credential = match &self.auth {
            None | Some(Auth::None) => Credential::None,
            Some(Auth::Bearer { token_env }) => {
                let token = read(token_env)?;
                // RFC 6750 b64token, with trailing padding only.
                let unpadded = token.trim_end_matches('=');
                if unpadded.is_empty()
                    || !unpadded
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b))
                {
                    bail!(
                        "credential environment variable {token_env} is not a valid bearer token"
                    );
                }
                Credential::Header {
                    name: "Authorization".into(),
                    value: format!("Bearer {token}"),
                }
            }
            Some(Auth::Basic {
                username_env,
                password_env,
            }) => {
                let username = read(username_env)?;
                if username.contains(':') {
                    bail!(
                        "credential environment variable {username_env} must not contain a colon"
                    );
                }
                let password = read(password_env)?;
                Credential::Header {
                    name: "Authorization".into(),
                    value: format!(
                        "Basic {}",
                        STANDARD.encode(format!("{username}:{password}"))
                    ),
                }
            }
            Some(Auth::ApiKey {
                location,
                name,
                value_env,
            }) => {
                let value = read(value_env)?;
                match location {
                    KeyLocation::Header => {
                        if ureq::http::header::HeaderValue::try_from(value.as_str()).is_err() {
                            bail!(
                                "credential environment variable {value_env} is not a valid HTTP header value"
                            );
                        }
                        Credential::Header {
                            name: name.clone(),
                            value,
                        }
                    }
                    KeyLocation::Query => Credential::Query {
                        name: name.clone(),
                        value,
                    },
                }
            }
        };
        Ok(RestSettings {
            base_url: Url::parse(&self.base_url).map_err(|_| anyhow!("invalid base_url"))?,
            capabilities: Capabilities {
                ipv4: self.supports_ipv4,
                ipv6: self.supports_ipv6,
                self_ip: self.request.self_path.is_some(),
            },
            request: self.request.clone(),
            response: self.response.clone(),
            credential,
        })
    }
}

fn validate_path(path: &str, allow_ip: bool) -> Result<bool> {
    if !path.starts_with('/')
        || path.contains(['?', '#', '%', '\\'])
        || path.chars().any(char::is_control)
    {
        bail!(
            "request paths must be absolute paths with unescaped segments (no query or fragment)"
        );
    }
    if path == "/" {
        return Ok(false);
    }
    let mut count = 0;
    for segment in path[1..].split('/') {
        if segment == "{ip}" && allow_ip {
            count += 1;
        } else if segment.is_empty()
            || matches!(segment, "." | "..")
            || !segment
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        {
            bail!("request path must contain only literal segments or a whole {{ip}} segment");
        }
    }
    if count > 1 {
        bail!("request path must contain at most one {{ip}} segment");
    }
    Ok(count == 1)
}

fn validate_query(query: &BTreeMap<String, String>, allow_ip: bool) -> Result<bool> {
    let mut count = 0;
    for (name, value) in query {
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b))
        {
            bail!(
                "request query parameter names must contain only ASCII letters, digits, '-', '.', '_' or '~'"
            );
        }
        if value == "{ip}" && allow_ip {
            count += 1;
        } else if value.contains(['{', '}']) || value.chars().any(char::is_control) {
            bail!("request query values must be literals or the whole {{ip}} placeholder");
        }
    }
    if count > 1 {
        bail!("request query must contain at most one {{ip}} value");
    }
    Ok(count == 1)
}

fn validate_pointer(pointer: &str) -> Result<()> {
    if (!pointer.is_empty() && !pointer.starts_with('/')) || pointer.chars().any(char::is_control) {
        bail!("JSON Pointers must be empty or start with '/' and contain no control characters");
    }
    for part in pointer.split('/').skip(1) {
        let mut chars = part.chars();
        while let Some(c) = chars.next() {
            if c == '~' && !matches!(chars.next(), Some('0' | '1')) {
                bail!("JSON Pointer contains an invalid '~' escape");
            }
        }
    }
    Ok(())
}

const TEXT_FIELDS: &[&str] = &[
    "ip",
    "network",
    "country_code",
    "country_name",
    "registered_country_code",
    "registered_country_name",
    "continent_code",
    "continent_name",
    "region",
    "city",
    "district",
    "postal_code",
    "timezone_name",
    "isp",
    "org",
    "asn_org",
    "asn",
    "connection_type",
    "hostname",
    "currency_code",
    "currency_name",
    "language_code",
    "language_name",
];
const NUMBER_FIELDS: &[&str] = &["latitude", "longitude", "asn_number", "accuracy_radius_km"];

impl Response {
    fn validate(&self) -> Result<()> {
        if self
            .expected_status
            .is_some_and(|status| !(200..=299).contains(&status))
        {
            bail!("response.expected_status must be a 2xx status");
        }
        if self.success_pointer.is_some() != self.success_equals.is_some() {
            bail!("response.success_pointer and success_equals must be provided together");
        }
        if self.success_pointer.is_none() && self.required_fields.is_empty() {
            bail!("response needs a success rule or required_fields");
        }
        for pointer in self
            .success_pointer
            .iter()
            .chain(self.error_message_pointer.iter())
            .chain(self.fields.values())
        {
            validate_pointer(pointer)?;
        }
        for name in self.fields.keys() {
            if !TEXT_FIELDS.contains(&name.as_str()) && !NUMBER_FIELDS.contains(&name.as_str()) {
                bail!("unknown response field {name:?}");
            }
        }
        let mut required = BTreeSet::new();
        for name in &self.required_fields {
            if !self.fields.contains_key(name) || !required.insert(name) {
                bail!("response.required_fields must name distinct mapped fields");
            }
        }
        for (name, transform) in &self.transforms {
            if !self.fields.contains_key(name)
                || !matches!(
                    (name.as_str(), transform),
                    ("asn_number", Transform::AsnToNumber)
                )
            {
                bail!(
                    "response.transforms only supports asn_number = 'asn_to_number' for a mapped field"
                );
            }
        }
        Ok(())
    }
}

impl Auth {
    fn validate(&self, request: &Request) -> Result<()> {
        match self {
            Self::None => {}
            Self::Bearer { token_env } => validate_env_name(token_env)?,
            Self::Basic {
                username_env,
                password_env,
            } => {
                validate_env_name(username_env)?;
                validate_env_name(password_env)?;
            }
            Self::ApiKey {
                location,
                name,
                value_env,
            } => {
                validate_env_name(value_env)?;
                match location {
                    KeyLocation::Header => {
                        if ureq::http::header::HeaderName::try_from(name.as_str()).is_err()
                            || [
                                "host",
                                "proxy-authorization",
                                "connection",
                                "transfer-encoding",
                                "content-length",
                            ]
                            .iter()
                            .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
                        {
                            bail!("invalid or forbidden API key header name");
                        }
                    }
                    KeyLocation::Query => {
                        validate_query(
                            &BTreeMap::from([(name.clone(), "key".to_string())]),
                            false,
                        )?;
                        if request.query.contains_key(name)
                            || request
                                .self_query
                                .as_ref()
                                .is_some_and(|query| query.contains_key(name))
                        {
                            bail!("API key query name conflicts with request query");
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

pub(crate) struct Client {
    transport: HttpTransport,
    settings: RestSettings,
}

impl Client {
    pub fn new(settings: RestSettings, timeout: Duration) -> Self {
        Self {
            transport: HttpTransport::configured(timeout),
            settings,
        }
    }

    #[cfg(test)]
    fn for_test(mut settings: RestSettings, base_url: &str) -> Self {
        // Only local tests can substitute an HTTP origin; production config is HTTPS-only.
        let url = Url::parse(base_url).unwrap();
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        settings.base_url = url;
        Self {
            transport: HttpTransport::configured_for_test(Duration::from_secs(1)),
            settings,
        }
    }

    fn url_for(&self, target: LookupTarget) -> Url {
        let ip = match target {
            LookupTarget::SelfIp => None,
            LookupTarget::Ip(ip) => Some(ip.to_string()),
        };
        let path = if ip.is_some() {
            &self.settings.request.path
        } else {
            self.settings
                .request
                .self_path
                .as_ref()
                .expect("self capability checked")
        };
        let query = if ip.is_some() {
            &self.settings.request.query
        } else {
            self.settings
                .request
                .self_query
                .as_ref()
                .unwrap_or(&self.settings.request.query)
        };
        let mut url = self.settings.base_url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .expect("HTTPS origin supports paths");
            segments.clear();
            if path != "/" {
                for segment in path[1..].split('/') {
                    segments.push(if segment == "{ip}" {
                        ip.as_deref().expect("validated template")
                    } else {
                        segment
                    });
                }
            }
        }
        if !query.is_empty() || matches!(self.settings.credential, Credential::Query { .. }) {
            let mut pairs = url.query_pairs_mut();
            for (name, value) in query {
                pairs.append_pair(
                    name,
                    if value == "{ip}" {
                        ip.as_deref().expect("validated template")
                    } else {
                        value
                    },
                );
            }
            if let Credential::Query { name, value } = &self.settings.credential {
                pairs.append_pair(name, value);
            }
        }
        url
    }

    fn lookup_response(&self, target: LookupTarget) -> Result<GeoRecord> {
        let url = self.url_for(target);
        let header = match &self.settings.credential {
            Credential::Header { name, value } => Some((name.as_str(), value.as_str())),
            _ => None,
        };
        let mut response = self.transport.get_configured(&url, header)?;
        let status = response.status().as_u16();
        if status != self.settings.response.expected_status.unwrap_or(200) {
            let reason = match status {
                401 => "authentication failed",
                402 => "quota exceeded",
                403 => "access denied",
                404 => "no geolocation data",
                429 => "rate limited",
                300..=399 => "redirect refused",
                _ => "request failed",
            };
            return Err(
                LookupError::http(status, format!("REST/JSON HTTP {status}: {reason}")).into(),
            );
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|header| header.to_str().ok())
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if content_type != "application/json"
            && !(content_type.starts_with("application/") && content_type.ends_with("+json"))
        {
            return Err(
                LookupError::new(ErrorCode::Decode, "REST/JSON response is not JSON").into(),
            );
        }
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY)
            .read_to_vec()
            .map_err(|error| {
                LookupError::from_http_client(
                    &error,
                    "REST/JSON response could not be read or exceeded size limit",
                )
            })?;
        let json: Value = serde_json::from_slice(&body)
            .map_err(|_| LookupError::new(ErrorCode::Decode, "invalid REST/JSON response JSON"))?;
        self.settings
            .response
            .map(&json)
            .map_err(|error| LookupError::from_error(&error, ErrorCode::Mapping).into())
    }
}

impl GeoProvider for Client {
    fn capabilities(&self) -> Capabilities {
        self.settings.capabilities
    }

    fn lookup(&self, target: LookupTarget) -> Result<LookupResult> {
        if !self.capabilities().supports(target) {
            return Err(LookupError::new(
                ErrorCode::UnsupportedCapability,
                "target is not supported by this profile",
            )
            .into());
        }
        Ok(LookupResult::new(self.lookup_response(target)?, true))
    }
}

impl Response {
    fn map(&self, json: &Value) -> Result<GeoRecord> {
        if let (Some(pointer), Some(expected)) = (&self.success_pointer, &self.success_equals) {
            let value = json
                .pointer(pointer)
                .ok_or_else(|| anyhow!("REST/JSON Mapping: success_pointer is missing"))?;
            if !expected.same_type(value) {
                bail!("REST/JSON Mapping: success_pointer has the wrong type");
            }
            if !expected.matches(value) {
                self.api_failure(json)?;
                return Err(
                    LookupError::new(ErrorCode::Api, "REST/JSON API reported failure").into(),
                );
            }
        }
        if let Some(pointer) = &self.error_message_pointer {
            // A present, nonempty error is an error even on HTTP 200; never print its
            // untrusted text, which can contain a query key or reflected token.
            match json.pointer(pointer) {
                Some(Value::String(message)) if !message.is_empty() => {
                    return Err(
                        LookupError::new(ErrorCode::Api, "REST/JSON API reported failure").into(),
                    );
                }
                None | Some(Value::Null | Value::String(_)) => {}
                _ => bail!("REST/JSON Mapping: error_message_pointer must refer to a string"),
            }
        }

        let mut record = GeoRecord::default();
        for (name, pointer) in &self.fields {
            let value = json.pointer(pointer).filter(|value| !value.is_null());
            if value.is_none() {
                if self.required_fields.contains(name) {
                    bail!("REST/JSON Mapping: missing required field {name}");
                }
                continue;
            }
            let value = value.expect("checked above");
            if TEXT_FIELDS.contains(&name.as_str()) {
                let text = value
                    .as_str()
                    .ok_or_else(|| anyhow!("REST/JSON Mapping: field {name} must be a string"))?;
                if text.is_empty() && self.required_fields.contains(name) {
                    bail!("REST/JSON Mapping: empty required field {name}");
                }
                match name.as_str() {
                    "ip" => record.ip = Some(text.into()),
                    "network" => record.network = Some(text.into()),
                    "country_code" => record.country_code = Some(text.into()),
                    "country_name" => record.country_name = Some(text.into()),
                    "registered_country_code" => record.registered_country_code = Some(text.into()),
                    "registered_country_name" => record.registered_country_name = Some(text.into()),
                    "continent_code" => record.continent_code = Some(text.into()),
                    "continent_name" => record.continent_name = Some(text.into()),
                    "region" => record.region = Some(text.into()),
                    "city" => record.city = Some(text.into()),
                    "district" => record.district = Some(text.into()),
                    "postal_code" => record.postal_code = Some(text.into()),
                    "timezone_name" => record.timezone_name = Some(text.into()),
                    "isp" => record.isp = Some(text.into()),
                    "org" => record.org = Some(text.into()),
                    "asn_org" => record.asn_org = Some(text.into()),
                    "asn" => record.asn = Some(text.into()),
                    "connection_type" => record.connection_type = Some(text.into()),
                    "hostname" => record.hostname = Some(text.into()),
                    "currency_code" => record.currency_code = Some(text.into()),
                    "currency_name" => record.currency_name = Some(text.into()),
                    "language_code" => record.language_code = Some(text.into()),
                    "language_name" => record.language_name = Some(text.into()),
                    _ => unreachable!("validated response field"),
                }
            } else {
                match name.as_str() {
                    "latitude" => record.latitude = Some(decimal(value, name)?),
                    "longitude" => record.longitude = Some(decimal(value, name)?),
                    "accuracy_radius_km" => {
                        record.accuracy_radius_km = Some(
                            value
                                .as_u64()
                                .and_then(|v| u32::try_from(v).ok())
                                .ok_or_else(|| {
                                    anyhow!(
                                        "REST/JSON Mapping: field {name} must be a nonnegative u32"
                                    )
                                })?,
                        )
                    }
                    "asn_number" => {
                        record.asn_number = Some(if self.transforms.contains_key(name) {
                            value
                                .as_str()
                                .and_then(|s| s.strip_prefix("AS"))
                                .and_then(|s| s.parse().ok())
                                .ok_or_else(|| {
                                    anyhow!("REST/JSON Mapping: field {name} must be an ASN string")
                                })?
                        } else {
                            value.as_u64().ok_or_else(|| {
                                anyhow!(
                                    "REST/JSON Mapping: field {name} must be a nonnegative integer"
                                )
                            })?
                        })
                    }
                    _ => unreachable!("validated response field"),
                }
            }
        }
        Ok(record)
    }

    fn api_failure(&self, json: &Value) -> Result<()> {
        if let Some(pointer) = &self.error_message_pointer {
            match json.pointer(pointer) {
                None | Some(Value::Null | Value::String(_)) => {}
                _ => bail!("REST/JSON Mapping: error_message_pointer must refer to a string"),
            }
        }
        Ok(())
    }
}

fn decimal(value: &Value, name: &str) -> Result<f64> {
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .ok_or_else(|| anyhow!("REST/JSON Mapping: field {name} must be a number"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Output;
    use crate::test_support::{Response as HttpResponse, TestServer};

    const PATH_PROFILE: &str = r#"
base_url = "https://fixture.invalid"
supports_ipv4 = true
supports_ipv6 = true
[auth]
type = "bearer"
token_env = "TEST_TOKEN"
[request]
method = "GET"
path = "/v1/lookup/{ip}"
self_path = "/v1/lookup/me"
[response]
success_pointer = "/ok"
success_equals = true
error_message_pointer = "/error/message"
required_fields = ["ip"]
[response.fields]
ip = "/data/ip"
country_name = "/data/country/name"
latitude = "/data/location/lat"
longitude = "/data/location/lon"
"#;

    const QUERY_PROFILE: &str = r#"
base_url = "https://fixture.invalid"
supports_ipv4 = true
supports_ipv6 = true
[auth]
type = "api_key"
location = "query"
name = "key"
value_env = "TEST_KEY"
[request]
method = "GET"
path = "/geo"
self_path = "/self"
[request.query]
ip = "{ip}"
mode = "full & city"
[request.self_query]
mode = "self view"
[response]
required_fields = ["country_code"]
[response.fields]
country_code = "/result/country"
asn_number = "/result/asn"
[response.transforms]
asn_number = "asn_to_number"
"#;

    fn profile(text: &str) -> RestProfile {
        let profile: RestProfile = toml::from_str(text).unwrap();
        profile.validate().unwrap();
        profile
    }

    fn client(text: &str, server: &TestServer, secret: &str) -> Client {
        let settings = profile(text).resolve_with(|_| Ok(secret.into())).unwrap();
        Client::for_test(settings, &server.url())
    }

    #[test]
    fn provider_label_names_rest_origin() {
        let settings = profile(PATH_PROFILE)
            .resolve_with(|_| Ok("secret-token".into()))
            .unwrap();
        assert_eq!(settings.provider_label(), "rest-json fixture.invalid");
    }

    #[test]
    fn first_api_path_bearer_mapping_and_self_lookup() {
        let server = TestServer::new(vec![
            HttpResponse::Http {
                status: 200,
                body: r#"{"ok":true,"data":{"ip":"2001:db8::1","country":{"name":"Test Country"},"location":{"lat":0,"lon":-122.5}}}"#,
            },
            HttpResponse::Http {
                status: 200,
                body: r#"{"ok":true,"data":{"ip":"8.8.8.8","country":{"name":null},"location":{"lat":0,"lon":0}}}"#,
            },
        ]);
        let client = client(PATH_PROFILE, &server, "secret-token");
        assert!(client.capabilities().supports(LookupTarget::SelfIp));
        let result = client
            .lookup(LookupTarget::Ip("2001:db8::1".parse().unwrap()))
            .unwrap();
        assert_eq!(result.record.country_name.as_deref(), Some("Test Country"));
        assert_eq!(result.record.latitude, Some(0.0));
        assert_eq!(result.record.longitude, Some(-122.5));
        let self_ip = client.lookup(LookupTarget::SelfIp).unwrap();
        assert_eq!(self_ip.record.ip.as_deref(), Some("8.8.8.8"));
        assert_eq!(self_ip.record.country_name, None);
        assert_eq!(
            server.requests(),
            [
                "GET /v1/lookup/2001:db8::1 HTTP/1.1",
                "GET /v1/lookup/me HTTP/1.1"
            ]
        );
        for headers in server.headers() {
            assert!(
                headers
                    .iter()
                    .any(|line| line.eq_ignore_ascii_case("authorization: Bearer secret-token"))
            );
        }
        let mut jsonl = Vec::new();
        Output::for_profile(&mut jsonl, true, "path-service")
            .emit(&result, "2001:db8::1", "2001:db8::1")
            .unwrap();
        let line: Value = serde_json::from_slice(&jsonl).unwrap();
        assert_eq!(line["schema_version"], 1);
        assert_eq!(line["profile"], "path-service");
        assert_eq!(line["data"]["latitude"], 0.0);
    }

    #[test]
    fn second_api_query_key_is_encoded_and_asn_is_explicitly_transformed() {
        let server = TestServer::new(vec![
            HttpResponse::HttpWithType {
                status: 200,
                content_type: "application/vnd.geo+json; charset=utf-8",
                body: r#"{"result":{"country":"AU","asn":"AS15169"}}"#,
            },
            HttpResponse::Http {
                status: 200,
                body: r#"{"result":{"country":"US"}}"#,
            },
        ]);
        let client = client(QUERY_PROFILE, &server, "a/? %&=+");
        let result = client
            .lookup(LookupTarget::Ip("2001:db8::1".parse().unwrap()))
            .unwrap();
        assert_eq!(result.record.country_code.as_deref(), Some("AU"));
        assert_eq!(result.record.asn_number, Some(15169));
        assert!(client.lookup(LookupTarget::SelfIp).unwrap().success);
        let requests = server.requests();
        assert!(requests[0].starts_with("GET /geo?"));
        assert!(requests[0].contains("ip=2001%3Adb8%3A%3A1"));
        assert!(requests[0].contains("mode=full+%26+city"));
        assert!(requests[0].contains("key=a%2F%3F+%25%26%3D%2B"));
        assert!(requests[1].starts_with("GET /self?mode=self+view&key="));
        assert!(!requests[1].contains("ip="));
    }

    #[test]
    fn self_only_profile_rejects_ip_without_an_http_request() {
        let text = PATH_PROFILE
            .replace(
                "supports_ipv4 = true\nsupports_ipv6 = true",
                "supports_ipv4 = false\nsupports_ipv6 = false",
            )
            .replace("path = \"/v1/lookup/{ip}\"", "path = \"/v1/lookup\"");
        let server = TestServer::new(vec![HttpResponse::Http {
            status: 200,
            body: r#"{"ok":true,"data":{"ip":"8.8.8.8"}}"#,
        }]);
        let client = client(&text, &server, "token");
        assert!(
            client
                .lookup(LookupTarget::Ip("1.1.1.1".parse().unwrap()))
                .is_err()
        );
        assert!(server.requests().is_empty());
        assert!(client.lookup(LookupTarget::SelfIp).unwrap().success);
        assert_eq!(server.requests(), ["GET /v1/lookup/me HTTP/1.1"]);
    }

    #[test]
    fn bad_json_mapping_and_api_failure_do_not_leak_untrusted_error_messages() {
        let server = TestServer::new(vec![
            HttpResponse::Http {
                status: 200,
                body: r#"{"ok":false,"error":{"message":"secret-token"}}"#,
            },
            HttpResponse::Http {
                status: 200,
                body: r#"{"ok":true,"data":{"ip":null}}"#,
            },
            HttpResponse::Http {
                status: 200,
                body: r#"{"ok":true,"data":{"ip":"1.1.1.1","location":{"lat":"0"}}}"#,
            },
            HttpResponse::Http {
                status: 200,
                body: r#"{"ok":true,"data":{"ip":"1.1.1.1","country":{"name":99}}}"#,
            },
            HttpResponse::Http {
                status: 200,
                body: r#"{"ok":"true","data":{"ip":"1.1.1.1"}}"#,
            },
            HttpResponse::Http {
                status: 200,
                body: r#"{"data":{"ip":"1.1.1.1"}}"#,
            },
            HttpResponse::Http {
                status: 200,
                body: r#"{"ok":true,"data":{"ip":""}}"#,
            },
        ]);
        let client = client(PATH_PROFILE, &server, "secret-token");
        for expected in [
            "reported failure",
            "missing required",
            "must be a number",
            "must be a string",
            "wrong type",
            "success_pointer is missing",
            "empty required",
        ] {
            let error = client.lookup(LookupTarget::SelfIp).unwrap_err();
            assert!(format!("{error:#}").contains(expected), "{error:#}");
            assert!(!format!("{error:#}").contains("secret-token"));
            assert_eq!(
                error.downcast_ref::<LookupError>().unwrap().code,
                if expected == "reported failure" {
                    ErrorCode::Api
                } else {
                    ErrorCode::Mapping
                }
            );
        }
    }

    #[test]
    fn rejects_redirects_without_forwarding_header_or_query_credentials() {
        for text in [PATH_PROFILE, QUERY_PROFILE] {
            let other = TestServer::new(vec![HttpResponse::Http {
                status: 200,
                body: "{}",
            }]);
            let server = TestServer::new(vec![HttpResponse::Redirect {
                location: other.url(),
            }]);
            let client = client(text, &server, "secret-key");
            let error = client
                .lookup(LookupTarget::Ip("8.8.8.8".parse().unwrap()))
                .unwrap_err();
            assert!(
                format!("{error:#}").contains("redirect")
                    || format!("{error:#}").contains("request failed")
            );
            assert!(!format!("{error:#}").contains("secret-key"));
            assert!(other.requests().is_empty());
        }
    }

    #[test]
    fn handles_http_and_decode_errors_with_bounded_body_and_no_secrets() {
        for (response, expected, code) in [
            (
                HttpResponse::Http {
                    status: 401,
                    body: "secret-token",
                },
                "authentication",
                ErrorCode::Authentication,
            ),
            (
                HttpResponse::Http {
                    status: 429,
                    body: "secret-token",
                },
                "rate limited",
                ErrorCode::RateLimited,
            ),
            (
                HttpResponse::HttpWithType {
                    status: 200,
                    body: "secret-token",
                    content_type: "text/html",
                },
                "not JSON",
                ErrorCode::Decode,
            ),
            (
                HttpResponse::Http {
                    status: 200,
                    body: "{secret-token}",
                },
                "invalid REST/JSON",
                ErrorCode::Decode,
            ),
            (
                HttpResponse::Http {
                    status: 200,
                    body: Box::leak(" ".repeat(MAX_BODY as usize + 1).into_boxed_str()),
                },
                "size limit",
                ErrorCode::Decode,
            ),
        ] {
            let server = TestServer::new(vec![response]);
            let client = client(PATH_PROFILE, &server, "secret-token");
            let error = client.lookup(LookupTarget::SelfIp).unwrap_err();
            assert_eq!(error.downcast_ref::<LookupError>().unwrap().code, code);
            let message = format!("{error:#}");
            assert!(message.contains(expected), "{message}");
            assert!(!message.contains("secret-token"));
        }
    }

    #[test]
    fn transport_failure_and_stalled_body_hide_query_key_and_request_url() {
        for (response, code) in [
            (HttpResponse::Disconnect, ErrorCode::Transport),
            (HttpResponse::StallBeforeHeaders, ErrorCode::Timeout),
            (HttpResponse::StallDuringBody, ErrorCode::Timeout),
        ] {
            let server = TestServer::new(vec![response]);
            let client = client(QUERY_PROFILE, &server, "key/should?not=leak");
            let error = client
                .lookup(LookupTarget::Ip("8.8.8.8".parse().unwrap()))
                .unwrap_err();
            let message = format!("{error:#}");
            assert_eq!(error.downcast_ref::<LookupError>().unwrap().code, code);
            assert!(!message.contains("key/should?not=leak"));
            assert!(!message.contains("key%2Fshould"));
            assert!(!message.contains("/geo?"));
        }
    }

    #[test]
    fn validates_templates_pointers_capabilities_and_auth_before_network() {
        for text in [
            PATH_PROFILE.replace("https://fixture.invalid", "http://fixture.invalid"),
            PATH_PROFILE.replace(
                "https://fixture.invalid",
                "https://user:secret@fixture.invalid",
            ),
            PATH_PROFILE.replace(
                "https://fixture.invalid",
                "https://fixture.invalid/?key=secret",
            ),
            PATH_PROFILE.replace("https://fixture.invalid", "https://fixture.invalid/extra"),
            PATH_PROFILE.replace("/v1/lookup/{ip}", "/v1/lookup/{ip}/../else"),
            PATH_PROFILE.replace("/v1/lookup/{ip}", "/v1/lookup/{ip}?key=secret"),
            PATH_PROFILE.replace("/v1/lookup/{ip}", "/v1/lookup/{ip}/{ip}"),
            PATH_PROFILE.replace("/v1/lookup/{ip}", "/v1/lookup/{ipa}"),
            PATH_PROFILE.replace("/v1/lookup/me", "/v1/lookup/{ip}"),
            PATH_PROFILE.replace(
                "supports_ipv4 = true\nsupports_ipv6 = true",
                "supports_ipv4 = false\nsupports_ipv6 = false",
            ),
            PATH_PROFILE.replace("/data/ip", "data/ip"),
            PATH_PROFILE.replace("/data/ip", "/bad~2pointer"),
            PATH_PROFILE.replace(
                "required_fields = [\"ip\"]",
                "required_fields = [\"country_code\"]",
            ),
            PATH_PROFILE.replace("ip = \"/data/ip\"", "unknown_field = \"/data/ip\""),
            PATH_PROFILE.replace("token_env = \"TEST_TOKEN\"", "token_env = \"INVALID=NAME\""),
            QUERY_PROFILE.replace("name = \"key\"", "name = \"ip\""),
            QUERY_PROFILE.replace("[request.self_query]\nmode = \"self view\"\n", ""),
            PATH_PROFILE.replace("success_equals = true\n", ""),
            PATH_PROFILE.replace("method = \"GET\"", "method = \"POST\""),
            PATH_PROFILE.replace("method = \"GET\"", "method = \"GET\"\nunknown = 1"),
        ] {
            let result: Result<RestProfile, _> = toml::from_str(&text);
            assert!(
                result.is_err() || result.unwrap().validate().is_err(),
                "accepted {text}"
            );
        }
    }

    #[test]
    fn basic_and_header_key_are_configurable_without_exposing_credentials() {
        let basic = PATH_PROFILE.replace(
            "type = \"bearer\"\ntoken_env = \"TEST_TOKEN\"",
            "type = \"basic\"\nusername_env = \"USER\"\npassword_env = \"PASS\"",
        );
        let settings = profile(&basic)
            .resolve_with(|name| Ok(if name == "USER" { "alice" } else { "password" }.into()))
            .unwrap();
        let server = TestServer::new(vec![HttpResponse::Http {
            status: 200,
            body: r#"{"ok":true,"data":{"ip":"8.8.8.8"}}"#,
        }]);
        Client::for_test(settings, &server.url())
            .lookup(LookupTarget::SelfIp)
            .unwrap();
        assert!(
            server.headers()[0]
                .iter()
                .any(|line| line.eq_ignore_ascii_case("authorization: Basic YWxpY2U6cGFzc3dvcmQ="))
        );

        let header_key = PATH_PROFILE.replace("type = \"bearer\"\ntoken_env = \"TEST_TOKEN\"", "type = \"api_key\"\nlocation = \"header\"\nname = \"X-API-Key\"\nvalue_env = \"TEST_KEY\"");
        let server = TestServer::new(vec![HttpResponse::Http {
            status: 200,
            body: r#"{"ok":true,"data":{"ip":"8.8.8.8"}}"#,
        }]);
        client(&header_key, &server, "secret-key")
            .lookup(LookupTarget::SelfIp)
            .unwrap();
        assert!(
            server.headers()[0]
                .iter()
                .any(|line| line.eq_ignore_ascii_case("x-api-key: secret-key"))
        );
        assert!(
            profile(&header_key)
                .resolve_with(|_| Err(anyhow!("missing secret")))
                .is_err()
        );

        let none = PATH_PROFILE.replace(
            "type = \"bearer\"\ntoken_env = \"TEST_TOKEN\"",
            "type = \"none\"",
        );
        let server = TestServer::new(vec![HttpResponse::Http {
            status: 200,
            body: r#"{"ok":true,"data":{"ip":"8.8.8.8"}}"#,
        }]);
        client(&none, &server, "unused")
            .lookup(LookupTarget::SelfIp)
            .unwrap();
        assert!(
            !server.headers()[0]
                .iter()
                .any(|line| line.to_ascii_lowercase().contains("authorization"))
        );
    }
}
