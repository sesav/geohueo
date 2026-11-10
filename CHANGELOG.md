# Changelog

## [Unreleased]

## [0.4.0]

### Added

- JSON includes `resolved_ip`, even when the provider leaves out the IP field.

### Changed

- **Breaking:** `--json` uses the same format for all providers. Lookup fields are now under `data`.
- **Breaking:** Each command accepts one IP address or hostname.

### Removed

- `--output normalized-json`. Use `--json` instead.

### Fixed

- Hostnames use a DNS address supported by the selected provider.

## [0.3.0]

### Added

- Ready-to-use binaries for Linux x86_64/ARM64, macOS Intel/Apple Silicon, and Windows x86_64.
- A Linux/macOS installer that checks downloads with SHA-256 and does not need `sudo`.
  You can choose the version and install folder.
- Installer tests and a manual workflow to test release builds without publishing them.

## [0.2.0]

### Added

- Library-level geolocation provider interface and a normalized geolocation model.
- TOML profiles for the existing geoiplookup.io provider, with CLI/environment selection and configuration validation.
- MaxMind GeoIP/GeoLite web-service profiles with Basic Auth, localized geolocation data, and normalized JSON Lines.
- Configurable REST/JSON profiles with URL templates, JSON Pointer field mapping, capability checks, and Bearer/Basic/API-key authentication.
- Explicit `--output normalized-json` for every profile; the existing geoiplookup.io `--json` format remains unchanged.
- Normalized JSON Lines preserve the original target and include structured errors with stable codes, diagnostic messages, and HTTP status when available.
- Typed provider errors distinguish timeouts, transport failures, authentication, permissions, quotas, decoding, mapping, and missing data.
- Validated example profiles and versioned JSONL contracts, covered by automated tests.

### Changed

- Default text output starts with the selected `Provider` and `Profile`, and each result uses indented `IP`/`Target` fields.
- Normalized JSON Lines include a top-level `provider` field alongside `profile`.
- Text output uses the resolved address in `IP` when a provider omits the IP field, including hostname lookups.

## [0.1.0]

Initial release.

### Added

- Geolocation lookup for the caller's public IP, IPv4/IPv6 addresses, and locally resolved hostnames.
- Multiple targets per invocation, with independent failure handling.
- IPv4/IPv6 hostname selection with `--ipv4` and `--ipv6`.
- Configurable HTTP request timeout with `--timeout`.
- Human-readable output and JSON Lines output with `--json`.
- Help, version information, and documented exit codes.

[Unreleased]: https://github.com/sesav/geohueo/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/sesav/geohueo/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/sesav/geohueo/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/sesav/geohueo/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/sesav/geohueo/releases/tag/v0.1.0
