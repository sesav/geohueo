# geolkp

A command-line utility for IPv4/IPv6 and hostname geolocation
via [geoiplookup.io](https://geoiplookup.io/api).

Intended solely for educational, non-commercial, personal home use.
The external service's terms of use apply.

```sh
cargo build --release
./target/release/geolkp 8.8.8.8
./target/release/geolkp --json example.com
```

Without an address, it looks up your public IP. Run `geolkp --help` for help.
