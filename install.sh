#!/bin/sh
# Install a verified release binary without Rust or elevated privileges.
set -eu

usage() {
    cat <<'EOF'
Usage: install.sh [--version VERSION] [--bin-dir DIR]

Install geohueo for Linux or macOS from GitHub Releases.

Options:
  --version VERSION  Install a specific version (with or without the v prefix)
  --bin-dir DIR      Installation directory (default: $HOME/.local/bin)
  -h, --help         Show this help

The default version is the latest stable release. Existing installations in the
selected directory are replaced only after the download passes SHA-256 verification.
Shell configuration files are never modified.
EOF
}

die() {
    printf 'geohueo installer: %s\n' "$*" >&2
    exit 1
}

curl_https() {
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --fail --silent --show-error --location --retry 3 \
        --connect-timeout 10 --max-time 120 "$@"
}

cleanup() {
    if [ -n "$staged_binary" ]; then
        rm -f "$staged_binary"
    fi
    if [ -n "$temp_dir" ]; then
        rm -rf "$temp_dir"
    fi
}

main() {
    version=
    bin_dir=
    temp_dir=
    staged_binary=
    repository=https://github.com/sesav/geohueo

    while [ "$#" -gt 0 ]; do
        case "$1" in
            --version)
                if [ "$#" -lt 2 ] || [ -z "$2" ]; then
                    die '--version requires a version'
                fi
                version=$2
                shift 2
                ;;
            --bin-dir)
                if [ "$#" -lt 2 ] || [ -z "$2" ]; then
                    die '--bin-dir requires a directory'
                fi
                bin_dir=$2
                shift 2
                ;;
            -h|--help)
                usage
                return
                ;;
            *) die "unknown argument: $1 (see --help)" ;;
        esac
    done

    for command in curl uname grep tar mktemp mkdir cp chmod mv rm; do
        command -v "$command" >/dev/null 2>&1 || die "required command not found: $command"
    done
    if command -v sha256sum >/dev/null 2>&1; then
        checksum_command=sha256sum
    elif command -v shasum >/dev/null 2>&1; then
        checksum_command=shasum
    else
        die 'SHA-256 verification requires sha256sum or shasum'
    fi

    os=$(uname -s)
    architecture=$(uname -m)
    case "$architecture" in
        x86_64|amd64) architecture=x86_64 ;;
        aarch64|arm64) architecture=aarch64 ;;
        *) die "unsupported architecture: $architecture; try cargo install geohueo --locked" ;;
    esac
    case "$os" in
        Linux) target=$architecture-unknown-linux-musl ;;
        Darwin) target=$architecture-apple-darwin ;;
        *) die "unsupported OS: $os; download an archive from $repository/releases" ;;
    esac

    if [ -z "$bin_dir" ]; then
        [ -n "${HOME:-}" ] || die 'HOME is not set; pass --bin-dir DIR'
        bin_dir=$HOME/.local/bin
    fi
    case "$bin_dir" in
        /*) ;;
        *) bin_dir=$(pwd)/$bin_dir ;;
    esac

    if [ -z "$version" ]; then
        latest_url=$(curl_https --head --output /dev/null --write-out '%{url_effective}' \
            "$repository/releases/latest") || die 'could not resolve the latest release; try --version VERSION'
        case "$latest_url" in
            "$repository"/releases/tag/*) version=${latest_url##*/} ;;
            *) die 'could not determine the latest release tag' ;;
        esac
    fi
    case "$version" in
        v*) ;;
        *) version=v$version ;;
    esac
    case "$version" in
        *[!0-9A-Za-z.+-]*) die "invalid version: $version" ;;
    esac
    printf '%s\n' "$version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$' \
        || die "invalid version: $version"

    archive=geohueo-$version-$target.tar.gz
    download_url=$repository/releases/download/$version
    trap cleanup EXIT
    trap 'exit 1' HUP INT TERM
    temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/geohueo.XXXXXXXX") || die 'could not create a temporary directory'

    printf 'Downloading geohueo %s for %s...\n' "$version" "$target"
    curl_https --output "$temp_dir/$archive" "$download_url/$archive" \
        || die "could not download $archive; check that the release provides this platform"
    curl_https --output "$temp_dir/$archive.sha256" "$download_url/$archive.sha256" \
        || die "could not download the checksum for $archive"

    read -r expected_checksum checksum_file < "$temp_dir/$archive.sha256" \
        || die 'invalid checksum file'
    [ "$checksum_file" = "$archive" ] || die 'checksum filename does not match the archive'
    [ "${#expected_checksum}" -eq 64 ] || die 'invalid SHA-256 checksum'
    case "$expected_checksum" in
        *[!0-9a-f]*) die 'invalid SHA-256 checksum' ;;
    esac
    if [ "$checksum_command" = sha256sum ]; then
        actual_checksum=$(sha256sum "$temp_dir/$archive") || die 'could not calculate SHA-256'
    else
        actual_checksum=$(shasum -a 256 "$temp_dir/$archive") || die 'could not calculate SHA-256'
    fi
    actual_checksum=${actual_checksum%% *}
    [ "$actual_checksum" = "$expected_checksum" ] || die 'SHA-256 checksum mismatch; nothing was installed'

    tar -xzf "$temp_dir/$archive" -C "$temp_dir" geohueo || die 'could not unpack the binary'
    if [ ! -f "$temp_dir/geohueo" ] || [ -L "$temp_dir/geohueo" ]; then
        die 'archive does not contain a regular geohueo binary'
    fi

    destination=$bin_dir/geohueo
    if [ -e "$destination" ] || [ -L "$destination" ]; then
        if [ ! -f "$destination" ] || [ -L "$destination" ]; then
            die "refusing to replace a non-regular file: $destination"
        fi
    fi
    mkdir -p "$bin_dir" || die "could not create $bin_dir"
    # Stage on the destination filesystem so replacing an existing binary is atomic.
    staged_binary=$(mktemp "$bin_dir/.geohueo.XXXXXXXX") || die "could not write to $bin_dir"
    cp "$temp_dir/geohueo" "$staged_binary" || die 'could not stage the binary'
    chmod 755 "$staged_binary" || die 'could not make the binary executable'
    mv -f "$staged_binary" "$destination" || die "could not install $destination"
    staged_binary=

    printf 'Installed geohueo %s to %s\n' "$version" "$destination"
    case ":${PATH:-}:" in
        *:"$bin_dir":*) ;;
        *) printf 'Add %s to your PATH to run geohueo by name.\n' "$bin_dir" ;;
    esac
}

# Keep execution last so a truncated download does not run a partial installer.
main "$@"
