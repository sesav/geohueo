# geolkp

A command-line utility for IPv4/IPv6 and hostname geolocation via [geoiplookup.io](https://geoiplookup.io/api).

Intended solely for educational, non-commercial, personal home use. The external service's terms of use apply.

## Build and usage

Requires Rust 1.85+.

```sh
cargo build --release
./target/release/geolkp                         # Your public IP
./target/release/geolkp 8.8.8.8 1.1.1.1          # Multiple addresses
./target/release/geolkp -4 --timeout 5 example.com
./target/release/geolkp --json example.com       # JSON Lines
```

`-4` / `-6` select IPv4 / IPv6 for hostname resolution only; IP literals and the API connection are unaffected.
`--timeout SECONDS` sets the HTTP timeout per request (positive whole seconds, default: 10), not the local hostname
resolution timeout. `--json` prints one JSON object per line. Run `geolkp --help` for all options.

Geolocation is approximate. Looked-up IP addresses are sent to the external service.

Exit codes: `0` — all lookups succeeded; `1` — resolution/request failure or no
geolocation data; `2` — invalid command-line arguments.
