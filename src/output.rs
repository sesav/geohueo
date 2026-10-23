use std::fmt::Write as _;
use std::io::{self, Write};

use crate::api::IpInfo;

pub struct Output<W> {
    writer: W,
    json: bool,
    first: bool,
}

impl<W: Write> Output<W> {
    pub fn new(writer: W, json: bool) -> Self {
        Self {
            writer,
            json,
            first: true,
        }
    }

    /// Failed lookups are still emitted in JSON mode, but not in text mode.
    pub fn emit(&mut self, info: &IpInfo, label: &str) -> io::Result<()> {
        if self.json {
            serde_json::to_writer(&mut self.writer, info)?;
            writeln!(self.writer)?;
        } else if info.success {
            if !self.first {
                writeln!(self.writer)?;
            }

            write!(self.writer, "{}", render(info, label))?;
            self.first = false;
        }

        Ok(())
    }
}

fn render(info: &IpInfo, label: &str) -> String {
    let mut out = String::new();
    out.push_str(label);
    out.push('\n');

    push_field(
        &mut out,
        "Country",
        &with_code(&info.country_name, &info.country_code),
    );
    push_field(
        &mut out,
        "Continent",
        &with_code(&info.continent_name, &info.continent_code),
    );
    push_field(&mut out, "Region", &info.region);
    push_field(&mut out, "City", &info.city);
    push_field(&mut out, "District", &info.district);
    push_field(&mut out, "Postal code", &info.postal_code);
    if let (Some(latitude), Some(longitude)) = (info.latitude, info.longitude) {
        push_field(
            &mut out,
            "Coordinates",
            &format!("{latitude:.4}, {longitude:.4}"),
        );
    }
    push_field(&mut out, "Timezone", &info.timezone_name);
    push_field(&mut out, "ISP", &info.isp);
    push_field(&mut out, "Organization", &info.org);
    push_field(&mut out, "ASN", &asn_label(info));
    push_field(&mut out, "Connection type", &info.connection_type);
    push_field(&mut out, "Hostname", &info.hostname);
    push_field(
        &mut out,
        "Currency",
        &with_code(&info.currency_name, &info.currency_code),
    );
    push_field(
        &mut out,
        "Language",
        &with_code(&info.language_name, &info.language_code),
    );

    out
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

fn asn_label(info: &IpInfo) -> String {
    if !info.asn.trim().is_empty() {
        return info.asn.trim().to_string();
    }

    let org = info.asn_org.trim();
    match (info.asn_number, org.is_empty()) {
        (0, true) => String::new(),
        (0, false) => org.to_string(),
        (number, true) => format!("AS{number}"),
        (number, false) => format!("AS{number} - {org}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_available_fields() {
        let info = IpInfo {
            ip: "8.8.8.8".to_string(),
            country_name: "United States".to_string(),
            country_code: "US".to_string(),
            city: "Mountain View".to_string(),
            latitude: Some(37.422),
            longitude: Some(-122.085),
            success: true,
            ..IpInfo::default()
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
        let mut info = IpInfo::default();
        assert!(!render(&info, "test").contains("Coordinates:"));

        info.latitude = Some(0.0);
        assert!(!render(&info, "test").contains("Coordinates:"));

        info.longitude = Some(0.0);
        assert!(render(&info, "test").contains("0.0000, 0.0000"));
    }

    #[test]
    fn formats_asn_fallback_and_codes() {
        let mut info = IpInfo {
            asn_number: 15169,
            asn_org: " Google LLC ".to_string(),
            ..IpInfo::default()
        };
        assert_eq!(asn_label(&info), "AS15169 - Google LLC");
        info.asn = " AS15169 ".to_string();
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

        output.emit(&info, "failed").unwrap();
        info.success = true;
        output.emit(&info, "first").unwrap();
        output.emit(&info, "second").unwrap();

        assert_eq!(String::from_utf8(bytes).unwrap(), "first\n\nsecond\n");
    }

    #[test]
    fn emits_one_json_object_per_line_including_failed_lookups() {
        let mut bytes = Vec::new();
        let mut output = Output::new(&mut bytes, true);
        let mut info = IpInfo::default();

        output.emit(&info, "failed").unwrap();
        info.success = true;
        info.city = "a\nb".to_string();
        output.emit(&info, "successful").unwrap();

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
                .emit(&info, "test")
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
        }
    }
}
