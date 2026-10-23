# geohueo

A command-line tool for finding IPv4/IPv6 and hostname locations using [geoiplookup.io](https://geoiplookup.io/api)
or MaxMind GeoIP/GeoLite web services.

Since there wasn't a clear, easy tool for domain or IP info, I just built a small utility for fun. Hoping it helps
others as well.

## Build

Requires Rust 1.85+.

```sh
cargo build --release
```

The binary is `target/release/geohueo`. Run the examples below from the repository root.

## Installation

Choose one of the following methods to make `geohueo` available from any directory. None requires `sudo`.

### Using Cargo (recommended)

Install [geohueo from crates.io](https://crates.io/crates/geohueo) with Rust 1.85+:

```sh
cargo install geohueo --locked
```

To update an installed copy to the latest published version, run the same command again.

To install from a source checkout instead, run from the repository root:

```sh
cargo install --path . --locked
```

Both commands build the release binary and install it for your user; no separate build is needed.

The default destination is `~/.cargo/bin` (or `$CARGO_HOME/bin` if `CARGO_HOME` is set).
If that directory is not already on your `PATH`, run:

```sh
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
```

### Installing the built binary manually (Linux/macOS)

After building, copy the binary to your user's local executable directory:

```sh
mkdir -p "$HOME/.local/bin"
install -m 755 target/release/geohueo "$HOME/.local/bin/geohueo"
export PATH="$HOME/.local/bin:$PATH"
```

To keep a `PATH` change across terminal sessions, add the relevant `export` line to your shell's startup file, such as
`~/.bashrc` or `~/.zshrc`.

### Verify the installation

```sh
geohueo --version
geohueo --help
```

After installation, you can use `geohueo` instead of `./target/release/geohueo` in all examples below.

## Configuration (source builds)

The current source version supports named profiles for **geoiplookup.io**, **MaxMind GeoIP** and **MaxMind GeoLite**
web services. Other API adapters are planned. Without a configuration file, the CLI behaves as before.

Create `$XDG_CONFIG_HOME/geohueo/config.toml` (or `~/.config/geohueo/config.toml` on Linux if `XDG_CONFIG_HOME` is unset):

```toml
schema_version = 1
default_profile = "personal"
timeout_seconds = 10

[profiles.personal]
adapter = "geoiplookup"
timeout_seconds = 5
```

```sh
geohueo --profile personal 8.8.8.8
geohueo --check-config
geohueo --list-profiles
geohueo --config ./config.toml --check-config
```

`GEOHUEO_CONFIG` selects the file and `GEOHUEO_PROFILE` selects a profile. Explicit `--config` and `--profile` take
precedence. Request timeout priority is `--timeout`, profile `timeout_seconds`, global `timeout_seconds`, then 10 seconds.
An absent default config file is allowed; a missing file explicitly selected with `--config` or `GEOHUEO_CONFIG` is an
error (exit code 2). `--check-config` and `--list-profiles` do not make API requests. Help and version work even when
the config is invalid. The built-in `geoiplookup` profile is always available.

### MaxMind GeoIP / GeoLite

Add a MaxMind profile to the TOML file (and optionally select it with `default_profile = "maxmind"`):

```toml
[profiles.maxmind]
adapter = "maxmind"
service = "geoip"           # "geoip" or "geolite"
product = "city"            # "country", "city", or "insights" (GeoIP only)
locales = ["ru", "en"]

[profiles.maxmind.auth]
type = "basic"
username_env = "MAXMIND_ACCOUNT_ID"
password_env = "MAXMIND_LICENSE_KEY"
```

Provide your **MaxMind account ID** and **license key** through these environment variables using your secret manager;
do not put them in the TOML file or on the command line. Only the selected profile's credentials are read.

```sh
geohueo --profile maxmind --check-config
geohueo --profile maxmind 8.8.8.8
geohueo --profile maxmind -6 example.com
geohueo --profile maxmind               # looks up the caller via MaxMind's /me endpoint
```

MaxMind GeoIP uses `geoip.maxmind.com`; GeoLite uses `geolite.info`. All requests use HTTPS and HTTP Basic Auth;
authentication is not forwarded on redirects. Product access and usage limits depend on your MaxMind account.
`--check-config` does not contact the service. A successful config check does not verify credentials or product access
with MaxMind.

## Usage examples

### Your public IP

With no address, the API looks up the caller's public IP as seen by the service.

```sh
./target/release/geohueo
```

### An IPv4 or IPv6 address

Look up an IPv4 address:

```sh
./target/release/geohueo 8.8.8.8
```

Or an IPv6 address:

```sh
./target/release/geohueo 2001:4860:4860::8888
```

### A hostname

The hostname is resolved locally, then its first returned IP address is looked up.

```sh
./target/release/geohueo example.com
```

### Multiple targets

Mix addresses and hostnames in one command. A failed lookup does not stop the remaining targets.

```sh
./target/release/geohueo 8.8.8.8 1.1.1.1 example.com
```

### Choose IPv4 or IPv6 for a hostname

Use `-4` (or `--ipv4`) to select the first IPv4 address:

```sh
./target/release/geohueo -4 example.com
```

Use `-6` (or `--ipv6`) to select the first IPv6 address:

```sh
./target/release/geohueo -6 example.com
```

These options cannot be combined. They affect hostname resolution only, not literal IP addresses or the API connection.
If the hostname has no address of the requested family, the lookup fails.

### Set the HTTP timeout

Limit each API request to five seconds:

```sh
./target/release/geohueo --timeout 5 example.com
```

The default is 10 seconds. Only positive whole seconds are accepted; `--timeout=5` also works.
This timeout does not cover local resolution of the input hostname.

### JSON output for scripts

Use `--json` (or `-j`) to print one JSON object per line (JSON Lines):

```sh
./target/release/geohueo --json 8.8.8.8
```

Save multiple results to a file:

```sh
./target/release/geohueo --json 8.8.8.8 1.1.1.1 > results.jsonl
```

Diagnostics go to stderr. Decoded API responses with `success: false` are still printed in JSON mode;
DNS, network, HTTP, or response decoding failures do not produce a JSON object.

For MaxMind, `--json` emits **normalized, versioned JSON Lines** instead of the legacy geoiplookup.io fields. Each
target produces a line, including errors, with `schema_version`, `profile`, `target`, `success`, and either `data` or
`error`. MaxMind Country responses may lack city, ISP and coordinates. Where provided, `accuracy_radius_km` represents
the approximate uncertainty of coordinates. The original geoiplookup.io `--json` format is unchanged.

### Help and version

Show all options:

```sh
./target/release/geohueo --help
```

Show the binary's version:

```sh
./target/release/geohueo --version
```

## Notes and exit codes

Geolocation is approximate, and looked-up IP addresses are sent to the external service.
Private, loopback, and other special-use addresses may have no geolocation data.

| Code | Meaning |
| --- | --- |
| `0` | All lookups succeeded. |
| `1` | A lookup failed or no geolocation data was found. |
| `2` | Invalid command-line arguments or configuration. |

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
```

Tests use local HTTP fixtures and do not query the public geolocation API.

Changes are listed in [CHANGELOG.md](CHANGELOG.md).

## License and API terms

The source code is distributed under the [MIT License](LICENSE), which permits commercial use of the code.

The geoiplookup.io service has separate [usage terms](https://geoiplookup.io/api). Its free API is for personal
and project use; commercial API use requires permission from the service provider. Contact
[post@octosec.io](mailto:post@octosec.io) for commercial usage. The MIT license does not grant rights to use the
external service or its data beyond the provider's terms. This is an unofficial client, not affiliated with
geoiplookup.io.

MaxMind services require a separate MaxMind account and are subject to [MaxMind's terms](https://www.maxmind.com/en/site-license-overview).
This is an unofficial client; the MIT license does not grant access to MaxMind data.
