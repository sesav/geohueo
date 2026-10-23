# geohueo

geohueo is a command-line tool for looking up geolocation and network information for IP addresses and hostnames.
It uses geoiplookup.io by default and requires no configuration or API credentials to get started.

Provider profiles support MaxMind's free GeoLite and paid GeoIP web services, as well as custom HTTP GET APIs that
return JSON. Configure profiles in TOML, select one per lookup, or set a default provider for subsequent lookups.

## Install

Requires Rust 1.85+.

```sh
cargo install geohueo --locked
```

From a source checkout:

```sh
cargo install --path . --locked
```

For development, `cargo run -- 8.8.8.8` also works from the checkout.

The binary is installed to `~/.cargo/bin`. Add it to your `PATH` if needed:

```sh
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
```

## Usage

```sh
geohueo 8.8.8.8
geohueo example.com
geohueo                  # this machine's public IP
geohueo 8.8.8.8 1.1.1.1 # several targets
geohueo -4 example.com   # IPv4 only
geohueo -6 example.com   # IPv6 only
geohueo --timeout 5 example.com
```

### geoiplookup.io (default)

```sh
geohueo 1.1.1.1
```

prints:

```text
  Provider:        geoiplookup.io
  Profile:         geoiplookup
  IP:              1.1.1.1
  Country:         Australia (AU)
  Continent:       Oceania (OC)
  Region:          New South Wales
  City:            Sydney
  Coordinates:     -33.8688, 151.2090
  Timezone:        Australia/Sydney
  ISP:             Cloudflare, Inc.
  ASN:             AS13335 - Cloudflare, Inc.
  Connection type: Corporate
  Currency:        Australian Dollar (AUD)
  Language:        English (en)
```

JSON output:

```sh
geohueo --json 1.1.1.1
```

prints:

```json
{
  "ip": "1.1.1.1",
  "isp": "Cloudflare, Inc.",
  "org": "",
  "hostname": "",
  "latitude": -33.8688,
  "longitude": 151.209,
  "postal_code": "",
  "city": "Sydney",
  "country_code": "AU",
  "country_name": "Australia",
  "continent_code": "OC",
  "continent_name": "Oceania",
  "region": "New South Wales",
  "district": "",
  "timezone_name": "Australia/Sydney",
  "connection_type": "Corporate",
  "asn_number": 13335,
  "asn_org": "Cloudflare, Inc.",
  "asn": "AS13335 - Cloudflare, Inc.",
  "currency_code": "AUD",
  "currency_name": "Australian Dollar",
  "language_code": "en",
  "language_name": "English",
  "success": true,
  "premium": false
}
```

### MaxMind GeoLite City

With a configured GeoLite City profile:

```sh
geohueo --profile geolite 8.8.8.8
```

prints:

```text
  Provider:        maxmind geolite city
  Profile:         geolite
  IP:              8.8.8.8
  Country:         United States (US)
  Continent:       North America (NA)
  Coordinates:     37.7510, -97.8220 (±1000 km)
  Timezone:        America/Chicago
  ASN:             AS15169 - Google LLC
```

JSON output:

```sh
geohueo --profile geolite --json 8.8.8.8
```

prints:

```json
{
  "schema_version": 1,
  "profile": "geolite",
  "provider": "maxmind geolite city",
  "target": "8.8.8.8",
  "success": true,
  "data": {
    "ip": "8.8.8.8",
    "network": "8.8.8.0/24",
    "country_code": "US",
    "country_name": "United States",
    "registered_country_code": "US",
    "registered_country_name": "United States",
    "continent_code": "NA",
    "continent_name": "North America",
    "region": null,
    "subdivisions": [],
    "city": null,
    "district": null,
    "postal_code": null,
    "latitude": 37.751,
    "longitude": -97.822,
    "accuracy_radius_km": 1000,
    "timezone_name": "America/Chicago",
    "isp": null,
    "org": null,
    "asn_number": 15169,
    "asn_org": "Google LLC",
    "asn": null,
    "connection_type": null,
    "hostname": null,
    "currency_code": null,
    "currency_name": null,
    "language_code": null,
    "language_name": null
  }
}
```

The JSON examples are pretty-printed for readability; geohueo writes one compact JSON object per line.

`--json` keeps the original format for geoiplookup and uses normalized JSON for MaxMind and REST/JSON profiles. To force normalized JSON for any profile (including geoiplookup):

```sh
geohueo --output normalized-json 8.8.8.8
```

`--json` and `--output` cannot be combined.

For hostnames, `IP` is the resolved address and `Target` is the hostname.

## Configuration

The built-in profile works without a configuration file. To add another provider, create a configuration file:

- Linux: `${XDG_CONFIG_HOME:-$HOME/.config}/geohueo/config.toml`
- macOS: `${XDG_CONFIG_HOME:-$HOME/Library/Application Support}/geohueo/config.toml`
- Windows: `%APPDATA%\geohueo\config.toml`

### MaxMind

A MaxMind account is required. GeoLite web services are free; GeoIP web services are paid. [Create a free MaxMind
account](https://www.maxmind.com/en/geolite2/signup), then use your Account ID and a license key from that account.

```toml
schema_version = 1
default_profile = "geolite"

[profiles.geolite]
adapter = "maxmind"
service = "geolite" # geoip or geolite
product = "city"    # country, city, or insights (geoip only)

[profiles.geolite.auth]
type = "basic"
username_env = "MAXMIND_ACCOUNT_ID"
password_env = "MAXMIND_LICENSE_KEY"
```

`username_env` and `password_env` are the **names** of environment variables, not the credentials themselves:

```sh
export MAXMIND_ACCOUNT_ID='your-account-id'
export MAXMIND_LICENSE_KEY='your-license-key'

geohueo --check-config
geohueo --profile geolite 8.8.8.8
```

`--check-config` validates the configuration and loads credentials for the selected profile without contacting the
provider. Ready-to-edit examples: [GeoIP
City](examples/profiles/maxmind-geoip.toml), [GeoLite City](examples/profiles/maxmind-geolite-city.toml), and [GeoLite
Country](examples/profiles/maxmind-geolite.toml).

### Other JSON APIs

Start from [rest-path.toml](examples/profiles/rest-path.toml) or [rest-query.toml](examples/profiles/rest-query.toml).
Copy one into your config, then change `base_url`, the request path/query, the JSON field pointers, and the auth
section. The examples cover Bearer, Basic, and API-key authentication.

```sh
geohueo --config ./config.toml --check-config
```

Use `--config PATH` or `GEOHUEO_CONFIG` to select a file, and `--profile NAME` or `GEOHUEO_PROFILE` to select a profile.
CLI flags take precedence. `--list-profiles` shows all profiles.

Keep the config and any credential file private (`chmod 600`).

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Lookup failed or no geolocation data |
| 2 | Invalid arguments or configuration, missing credentials, or unsupported target |

Geolocation is approximate. Private, loopback, and other special addresses may have no data. Looked-up addresses are
sent to the selected external service.

## License

The code is MIT licensed. geoiplookup.io, MaxMind, and custom services have their own terms and may require an account
or payment. This is an unofficial client, not affiliated with geoiplookup.io or MaxMind.

- [MIT License](LICENSE)
- [geoiplookup.io API terms](https://geoiplookup.io/api)
- [MaxMind terms](https://www.maxmind.com/en/site-license-overview)
- [Changelog](CHANGELOG.md)
