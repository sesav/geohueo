use std::fmt::Write as _;
use std::io::{self, Write};
use std::net::IpAddr;

use serde::Serialize;

use crate::error::{ErrorCode, LookupError};
use crate::model::{GeoRecord, LookupResult};

pub struct Output<W> {
    writer: W,
    json: bool,
    header_printed: bool,
    profile: Option<String>,
    provider: Option<String>,
}

impl<W: Write> Output<W> {
    pub fn new(writer: W, json: bool) -> Self {
        Self {
            writer,
            json,
            header_printed: false,
            profile: None,
            provider: None,
        }
    }

    pub fn for_profile(writer: W, json: bool, profile: &str) -> Self {
        let mut output = Self::new(writer, json);
        output.profile = Some(profile.to_string());
        output
    }

    pub fn set_provider(&mut self, provider: &str) {
        self.provider = Some(provider.to_string());
    }

    /// Text output starts with the selected provider and profile, before any
    /// lookup result. This is also visible when every lookup fails.
    pub fn write_header(&mut self) -> io::Result<()> {
        if self.json || self.header_printed {
            return Ok(());
        }
        let Some(profile) = self.profile.as_deref() else {
            return Ok(());
        };

        let mut header = String::new();
        push_field(
            &mut header,
            "Provider",
            self.provider.as_deref().unwrap_or(""),
        );
        push_field(&mut header, "Profile", profile);
        write!(self.writer, "{header}")?;
        self.header_printed = true;
        Ok(())
    }

    /// All providers use the same JSON format.
    pub fn emit(
        &mut self,
        result: &LookupResult,
        target: &str,
        resolved_ip: Option<IpAddr>,
    ) -> io::Result<()> {
        if self.json {
            if result.success {
                self.write_normalized(target, resolved_ip, true, Some(&result.record), None)?;
            } else {
                self.write_normalized(
                    target,
                    resolved_ip,
                    false,
                    None,
                    Some(&LookupError::new(ErrorCode::NoData, "no geolocation data")),
                )?;
            }
        } else if result.success {
            let ip = resolved_ip.map(|ip| ip.to_string());
            write!(
                self.writer,
                "{}",
                render(&result.record, ip.as_deref().unwrap_or(target), target)
            )?;
        }

        Ok(())
    }

    pub fn emit_error(
        &mut self,
        target: &str,
        resolved_ip: Option<IpAddr>,
        error: &LookupError,
    ) -> io::Result<()> {
        if self.json {
            self.write_normalized(target, resolved_ip, false, None, Some(error))?;
        }
        Ok(())
    }

    fn write_normalized(
        &mut self,
        target: &str,
        resolved_ip: Option<IpAddr>,
        success: bool,
        data: Option<&GeoRecord>,
        error: Option<&LookupError>,
    ) -> io::Result<()> {
        #[derive(Serialize)]
        struct Line<'a> {
            schema_version: u8,
            profile: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            provider: Option<&'a str>,
            target: &'a str,
            resolved_ip: Option<IpAddr>,
            success: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            data: Option<&'a GeoRecord>,
            #[serde(skip_serializing_if = "Option::is_none")]
            error: Option<&'a LookupError>,
        }

        let profile = self.profile.as_deref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "provider has no JSON output")
        })?;
        serde_json::to_writer(
            &mut self.writer,
            &Line {
                schema_version: 1,
                profile,
                provider: self.provider.as_deref(),
                target,
                resolved_ip,
                success,
                data,
                error,
            },
        )?;
        writeln!(self.writer)
    }
}

fn render(info: &GeoRecord, resolved_ip: &str, target: &str) -> String {
    let mut out = String::new();
    let ip = info
        .ip
        .as_deref()
        .filter(|ip| !ip.is_empty())
        .unwrap_or(resolved_ip);

    push_field(&mut out, "IP", ip);
    if target != ip && target != "current IP" {
        push_field(&mut out, "Target", target);
    }
    push_field(
        &mut out,
        "Country",
        &with_code(field(&info.country_name), field(&info.country_code)),
    );
    push_field(
        &mut out,
        "Continent",
        &with_code(field(&info.continent_name), field(&info.continent_code)),
    );
    push_field(&mut out, "Region", field(&info.region));
    push_field(&mut out, "City", field(&info.city));
    push_field(&mut out, "District", field(&info.district));
    push_field(&mut out, "Postal code", field(&info.postal_code));
    if let (Some(latitude), Some(longitude)) = (info.latitude, info.longitude) {
        push_field(
            &mut out,
            "Coordinates",
            &match info.accuracy_radius_km {
                Some(radius) => format!("{latitude:.4}, {longitude:.4} (±{radius} km)"),
                None => format!("{latitude:.4}, {longitude:.4}"),
            },
        );
    }
    push_field(&mut out, "Timezone", field(&info.timezone_name));
    push_field(&mut out, "ISP", field(&info.isp));
    push_field(&mut out, "Organization", field(&info.org));
    push_field(&mut out, "ASN", &asn_label(info));
    push_field(&mut out, "Connection type", field(&info.connection_type));
    push_field(&mut out, "Hostname", field(&info.hostname));
    push_field(
        &mut out,
        "Currency",
        &with_code(field(&info.currency_name), field(&info.currency_code)),
    );
    push_field(
        &mut out,
        "Language",
        &with_code(field(&info.language_name), field(&info.language_code)),
    );

    out
}

fn field(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("")
}

fn push_field(out: &mut String, label: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }

    let label = format!("{label}:");
    let _ = writeln!(out, "  {label:<16} {value}");
}

fn with_code(name: &str, code: &str) -> String {
    match (name.trim(), code.trim()) {
        ("", "") => String::new(),
        (name, "") => name.to_string(),
        ("", code) => code.to_string(),
        (name, code) => format!("{name} ({code})"),
    }
}

fn asn_label(info: &GeoRecord) -> String {
    let asn = field(&info.asn).trim();
    if !asn.is_empty() {
        return asn.to_string();
    }

    let org = field(&info.asn_org).trim();
    match (info.asn_number.unwrap_or(0), org.is_empty()) {
        (0, true) => String::new(),
        (0, false) => org.to_string(),
        (number, true) => format!("AS{number}"),
        (number, false) => format!("AS{number} - {org}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::geoiplookup::IpInfo;

    fn legacy_result(info: &IpInfo) -> LookupResult {
        LookupResult::new(GeoRecord::from(info), info.success)
            .with_legacy_json(serde_json::to_string(info).unwrap())
    }

    fn jsonl_text(bytes: &[u8]) -> String {
        std::str::from_utf8(bytes)
            .expect("JSONL fixture must be UTF-8")
            .replace("\r\n", "\n")
    }

    #[test]
    fn renders_available_fields() {
        let info = GeoRecord {
            ip: Some("8.8.8.8".to_string()),
            country_name: Some("United States".to_string()),
            country_code: Some("US".to_string()),
            city: Some("Mountain View".to_string()),
            latitude: Some(37.422),
            longitude: Some(-122.085),
            ..GeoRecord::default()
        };

        let output = render(&info, "8.8.8.8", "8.8.8.8");

        assert!(output.starts_with("  IP:"));
        assert!(output.lines().next().unwrap().ends_with("8.8.8.8"));
        assert!(output.contains("Country:"));
        assert!(output.contains("United States (US)"));
        assert!(output.contains("37.4220, -122.0850"));
        assert!(!output.contains("Timezone:"));
        assert!(!output.contains("ASN:"));
    }

    #[test]
    fn text_header_identifies_provider_and_profile() {
        let mut bytes = Vec::new();
        let mut output = Output::for_profile(&mut bytes, false, "maxmind-geolite");
        output.set_provider("maxmind geolite country");

        output.write_header().unwrap();
        output.write_header().unwrap();

        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("Provider:"));
        assert!(text.contains("maxmind geolite country"));
        assert!(text.contains("Profile:"));
        assert!(text.contains("maxmind-geolite"));
        assert_eq!(text.matches("Provider:").count(), 1);
        assert_eq!(text.matches("Profile:").count(), 1);
    }

    #[test]
    fn renders_hostname_target_and_resolved_ip_separately() {
        let info = GeoRecord {
            ip: Some("1.2.3.4".to_string()),
            ..GeoRecord::default()
        };

        let output = render(&info, "1.2.3.4", "example.com");

        assert!(output.contains("IP:"));
        assert!(output.contains("1.2.3.4"));
        assert!(output.contains("Target:"));
        assert!(output.contains("example.com"));
        assert!(!output.contains("example.com (1.2.3.4)"));
    }

    #[test]
    fn omits_missing_or_incomplete_coordinates() {
        let mut info = GeoRecord::default();
        assert!(!render(&info, "test", "test").contains("Coordinates:"));

        info.latitude = Some(0.0);
        assert!(!render(&info, "test", "test").contains("Coordinates:"));

        info.longitude = Some(0.0);
        assert!(render(&info, "test", "test").contains("0.0000, 0.0000"));
    }

    #[test]
    fn formats_asn_fallback_and_codes() {
        let mut info = GeoRecord {
            asn_number: Some(15169),
            asn_org: Some(" Google LLC ".to_string()),
            ..GeoRecord::default()
        };
        assert_eq!(asn_label(&info), "AS15169 - Google LLC");
        info.asn = Some(" AS15169 ".to_string());
        assert_eq!(asn_label(&info), "AS15169");
        assert_eq!(with_code(" US ", " "), "US");
        assert_eq!(with_code("", " US "), "US");
        assert_eq!(with_code(" ", ""), "");
    }

    #[test]
    fn text_output_has_no_extra_blank_lines() {
        for success in [false, true] {
            let mut bytes = Vec::new();
            let info = LookupResult::new(GeoRecord::default(), success);
            Output::new(&mut bytes, false)
                .emit(&info, "8.8.8.8", None)
                .unwrap();
            let expected = if success {
                render(&info.record, "8.8.8.8", "8.8.8.8")
            } else {
                String::new()
            };
            assert_eq!(String::from_utf8(bytes).unwrap(), expected);
        }
    }

    #[test]
    fn json_output_is_one_object_for_success_or_failure() {
        for success in [false, true] {
            let mut bytes = Vec::new();
            let info = IpInfo {
                success,
                city: "a\nb".into(),
                ..IpInfo::default()
            };
            Output::for_profile(&mut bytes, true, "geoiplookup")
                .emit(&legacy_result(&info), "8.8.8.8", None)
                .unwrap();
            let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(result["success"], success);
            if success {
                assert_eq!(result["data"]["city"], "a\nb");
            } else {
                assert_eq!(result["error"]["code"], "no_data");
                assert!(result.get("data").is_none());
            }
        }
    }

    #[test]
    fn geoiplookup_json_uses_the_normalized_schema_instead_of_its_legacy_payload() {
        let info = IpInfo {
            ip: "8.8.8.8".to_string(),
            latitude: Some(0.0),
            success: true,
            ..IpInfo::default()
        };
        let mut bytes = Vec::new();

        Output::for_profile(&mut bytes, true, "geoiplookup")
            .emit(
                &legacy_result(&info),
                "8.8.8.8",
                Some("8.8.8.8".parse().unwrap()),
            )
            .unwrap();

        let line: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(line["schema_version"], 1);
        assert_eq!(line["profile"], "geoiplookup");
        assert_eq!(line["target"], "8.8.8.8");
        assert_eq!(line["resolved_ip"], "8.8.8.8");
        assert_eq!(line["data"]["latitude"], 0.0);
        assert!(line.get("latitude").is_none());
        assert!(line.get("premium").is_none());
    }

    #[test]
    fn released_jsonl_contracts_match_shipped_examples() {
        let info = IpInfo {
            ip: "8.8.8.8".into(),
            country_code: "US".into(),
            success: true,
            ..IpInfo::default()
        };
        let mut geoiplookup = Vec::new();
        let mut output = Output::for_profile(&mut geoiplookup, true, "geoiplookup");
        output.set_provider("geoiplookup.io");
        output
            .emit(
                &legacy_result(&info),
                "8.8.8.8",
                Some("8.8.8.8".parse().unwrap()),
            )
            .unwrap();
        assert_eq!(
            jsonl_text(&geoiplookup),
            jsonl_text(include_bytes!(
                "../examples/jsonl/geoiplookup-success.jsonl"
            ))
        );

        let record = GeoRecord {
            ip: Some("8.8.8.8".into()),
            country_code: Some("US".into()),
            latitude: Some(0.0),
            longitude: Some(-122.1),
            ..GeoRecord::default()
        };
        let mut normalized = Vec::new();
        let mut output = Output::for_profile(&mut normalized, true, "maxmind");
        output.set_provider("maxmind geoip city");
        output
            .emit(
                &LookupResult::new(record, true),
                "8.8.8.8",
                Some("8.8.8.8".parse().unwrap()),
            )
            .unwrap();
        assert_eq!(
            jsonl_text(&normalized),
            jsonl_text(include_bytes!("../examples/jsonl/normalized-success.jsonl"))
        );

        let mut failure = Vec::new();
        let info = IpInfo {
            success: false,
            ..info
        };
        let mut output = Output::for_profile(&mut failure, true, "geoiplookup");
        output.set_provider("geoiplookup.io");
        output
            .emit(
                &legacy_result(&info),
                "8.8.8.8",
                Some("8.8.8.8".parse().unwrap()),
            )
            .unwrap();
        assert_eq!(
            jsonl_text(&failure),
            jsonl_text(include_bytes!("../examples/jsonl/normalized-error.jsonl"))
        );
    }

    #[test]
    fn normalized_errors_have_no_data_and_keep_escaped_input_per_line() {
        let mut bytes = Vec::new();
        let mut output = Output::for_profile(&mut bytes, true, "custom");
        output.set_provider("rest-json geo.example.invalid");
        output
            .emit_error(
                "bad\nname",
                None,
                &LookupError::new(ErrorCode::Dns, "cannot resolve"),
            )
            .unwrap();
        let lines: Vec<serde_json::Value> = String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0]["schema_version"], 1);
        assert_eq!(lines[0]["profile"], "custom");
        assert_eq!(lines[0]["provider"], "rest-json geo.example.invalid");
        assert_eq!(lines[0]["target"], "bad\nname");
        assert_eq!(lines[0]["error"]["message"], "cannot resolve");
        assert_eq!(lines[0]["error"]["code"], "dns");
        assert!(lines[0].get("data").is_none());
    }

    #[test]
    fn propagates_broken_pipe_in_both_formats() {
        struct BrokenWriter;

        impl Write for BrokenWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let info = IpInfo {
            success: true,
            ..IpInfo::default()
        };
        for json in [false, true] {
            let error = Output::for_profile(BrokenWriter, json, "geoiplookup")
                .emit(&legacy_result(&info), "test", None)
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        }
        let error = Output::for_profile(BrokenWriter, true, "maxmind")
            .emit(&LookupResult::new(GeoRecord::default(), true), "test", None)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
