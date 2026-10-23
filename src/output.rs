use std::fmt::Write as _;
use std::io::{self, Write};

use serde::Serialize;

use crate::model::{GeoRecord, LookupResult};

pub struct Output<W> {
    writer: W,
    json: bool,
    first: bool,
    profile: Option<String>,
}

impl<W: Write> Output<W> {
    pub fn new(writer: W, json: bool) -> Self {
        Self {
            writer,
            json,
            first: true,
            profile: None,
        }
    }

    pub fn for_profile(writer: W, json: bool, profile: &str) -> Self {
        let mut output = Self::new(writer, json);
        output.profile = Some(profile.to_string());
        output
    }

    /// Decoded legacy `success: false` responses are emitted in JSON mode only.
    pub fn emit(&mut self, result: &LookupResult, label: &str, target: &str) -> io::Result<()> {
        if self.json {
            if let Some(json) = result.legacy_json() {
                writeln!(self.writer, "{json}")?;
            } else {
                self.write_normalized(target, result.success, Some(&result.record), None)?;
            }
        } else if result.success {
            if !self.first {
                writeln!(self.writer)?;
            }

            write!(self.writer, "{}", render(&result.record, label))?;
            self.first = false;
        }

        Ok(())
    }

    pub fn emit_error(&mut self, target: &str, message: &str) -> io::Result<()> {
        if self.json && self.profile.is_some() {
            self.write_normalized(target, false, None, Some(message))?;
        }
        Ok(())
    }

    fn write_normalized(
        &mut self,
        target: &str,
        success: bool,
        data: Option<&GeoRecord>,
        error: Option<&str>,
    ) -> io::Result<()> {
        #[derive(Serialize)]
        struct Line<'a> {
            schema_version: u8,
            profile: &'a str,
            target: &'a str,
            success: bool,
            #[serde(skip_serializing_if = "Option::is_none")]
            data: Option<&'a GeoRecord>,
            #[serde(skip_serializing_if = "Option::is_none")]
            error: Option<&'a str>,
        }

        let profile = self.profile.as_deref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "provider has no JSON output")
        })?;
        serde_json::to_writer(
            &mut self.writer,
            &Line {
                schema_version: 1,
                profile,
                target,
                success,
                data,
                error,
            },
        )?;
        writeln!(self.writer)
    }
}

fn render(info: &GeoRecord, label: &str) -> String {
    let mut out = String::new();
    out.push_str(label);
    out.push('\n');

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

        let output = render(&info, "8.8.8.8");

        assert!(output.starts_with("8.8.8.8\n"));
        assert!(output.contains("Country:"));
        assert!(output.contains("United States (US)"));
        assert!(output.contains("37.4220, -122.0850"));
        assert!(!output.contains("Timezone:"));
        assert!(!output.contains("ASN:"));
    }

    #[test]
    fn omits_missing_or_incomplete_coordinates() {
        let mut info = GeoRecord::default();
        assert!(!render(&info, "test").contains("Coordinates:"));

        info.latitude = Some(0.0);
        assert!(!render(&info, "test").contains("Coordinates:"));

        info.longitude = Some(0.0);
        assert!(render(&info, "test").contains("0.0000, 0.0000"));
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
    fn separates_only_successful_text_results() {
        let mut bytes = Vec::new();
        let mut output = Output::new(&mut bytes, false);
        let mut info = IpInfo::default();

        output
            .emit(&legacy_result(&info), "failed", "failed")
            .unwrap();
        info.success = true;
        output
            .emit(&legacy_result(&info), "first", "first")
            .unwrap();
        output
            .emit(&legacy_result(&info), "second", "second")
            .unwrap();

        assert_eq!(String::from_utf8(bytes).unwrap(), "first\n\nsecond\n");
    }

    #[test]
    fn emits_one_json_object_per_line_including_failed_lookups() {
        let mut bytes = Vec::new();
        let mut output = Output::new(&mut bytes, true);
        let mut info = IpInfo::default();

        output
            .emit(&legacy_result(&info), "failed", "failed")
            .unwrap();
        info.success = true;
        info.city = "a\nb".to_string();
        output
            .emit(&legacy_result(&info), "successful", "successful")
            .unwrap();

        let text = String::from_utf8(bytes).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["success"], false);
        assert_eq!(lines[1]["success"], true);
        assert_eq!(lines[1]["city"], "a\nb");
    }

    #[test]
    fn legacy_json_output_matches_original_dto_serialization() {
        let info = IpInfo {
            ip: "8.8.8.8".to_string(),
            latitude: Some(0.0),
            success: true,
            ..IpInfo::default()
        };
        let mut bytes = Vec::new();

        Output::new(&mut bytes, true)
            .emit(&legacy_result(&info), "8.8.8.8", "8.8.8.8")
            .unwrap();

        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            format!("{}\n", serde_json::to_string(&info).unwrap())
        );
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
            let error = Output::new(BrokenWriter, json)
                .emit(&legacy_result(&info), "test", "test")
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        }
    }
}
