# Changelog

## [Unreleased]

## [0.2.0]

### Added

- Library-level geolocation provider interface and a normalized geolocation model.
- TOML profiles for the existing geoiplookup.io provider, with CLI/environment selection and configuration validation.
- MaxMind GeoIP/GeoLite web-service profiles with Basic Auth, localized geolocation data, and normalized JSON Lines.
- Configurable REST/JSON profiles with URL templates, JSON Pointer field mapping, capability checks, and Bearer/Basic/API-key authentication.
- Explicit `--output normalized-json` for every profile; the existing geoiplookup.io `--json` format remains unchanged.
- Validated example profiles and versioned JSONL contracts, covered by automated tests.

## [0.1.0]

Initial release.

### Added

- Geolocation lookup for the caller's public IP, IPv4/IPv6 addresses, and locally resolved hostnames.
- Multiple targets per invocation, with independent failure handling.
- IPv4/IPv6 hostname selection with `--ipv4` and `--ipv6`.
- Configurable HTTP request timeout with `--timeout`.
- Human-readable output and JSON Lines output with `--json`.
- Help, version information, and documented exit codes.

[Unreleased]: https://github.com/sesav/geohueo/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/sesav/geohueo/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/sesav/geohueo/releases/tag/v0.1.0
