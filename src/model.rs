use std::net::IpAddr;

use serde::Serialize;

/// An IP address, or the public address seen by the provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupTarget {
    SelfIp,
    Ip(IpAddr),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GeoSubdivision {
    pub code: Option<String>,
    pub name: Option<String>,
}

/// Fields shared by geolocation services. A missing field is distinct from a zero value.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct GeoRecord {
    pub ip: Option<String>,
    pub network: Option<String>,
    pub country_code: Option<String>,
    pub country_name: Option<String>,
    pub registered_country_code: Option<String>,
    pub registered_country_name: Option<String>,
    pub continent_code: Option<String>,
    pub continent_name: Option<String>,
    pub region: Option<String>,
    pub subdivisions: Vec<GeoSubdivision>,
    pub city: Option<String>,
    pub district: Option<String>,
    pub postal_code: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub accuracy_radius_km: Option<u32>,
    pub timezone_name: Option<String>,
    pub isp: Option<String>,
    pub org: Option<String>,
    pub asn_number: Option<u64>,
    pub asn_org: Option<String>,
    pub asn: Option<String>,
    pub connection_type: Option<String>,
    pub hostname: Option<String>,
    pub currency_code: Option<String>,
    pub currency_name: Option<String>,
    pub language_code: Option<String>,
    pub language_name: Option<String>,
}

/// A decoded lookup. Providers may attach an exact legacy JSON representation
/// so existing consumers keep receiving their original schema.
#[derive(Debug, Clone)]
pub struct LookupResult {
    pub record: GeoRecord,
    pub success: bool,
    legacy_json: Option<String>,
}

impl LookupResult {
    pub fn new(record: GeoRecord, success: bool) -> Self {
        Self {
            record,
            success,
            legacy_json: None,
        }
    }

    pub fn legacy_json(&self) -> Option<&str> {
        self.legacy_json.as_deref()
    }

    pub(crate) fn with_legacy_json(mut self, json: String) -> Self {
        self.legacy_json = Some(json);
        self
    }
}
