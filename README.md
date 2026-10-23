# geohueo

A command-line tool for finding IPv4/IPv6 and hostname locations using [geoiplookup.io](https://geoiplookup.io/api).
Just for personal, home, non-commercial, educational use. The outside service's rules apply here.

Since there wasn't a clear, easy tool for domain or IP info, I just built a small utility for fun. Hoping it helps
others as well.

## Build

Requires Rust 1.85+.

```sh
cargo build --release
```

The binary is `target/release/geohueo`. Run the examples below from the repository root.

## Installation

Choose one of the following methods to make `geohueo` available from any directory. Neither requires `sudo`.

### Using Cargo (recommended)

Run from the repository root. Cargo builds the release binary and installs it for your user;
no separate build is needed.

```sh
cargo install --path . --locked
```

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
| `2` | Invalid command-line arguments. |
