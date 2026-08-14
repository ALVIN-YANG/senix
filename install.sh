#!/bin/sh
set -eu

repository=${SENIX_REPOSITORY:-ALVIN-YANG/senix}
version=${SENIX_VERSION:-}
install_dir=${SENIX_INSTALL_DIR:-/usr/local/bin}
release_base_url=${SENIX_RELEASE_BASE_URL:-https://github.com/${repository}/releases}
release_base_url=${release_base_url%/}

usage() {
  cat <<'EOF'
Install the latest Senix release from GitHub.

Usage: install.sh [--version v0.3.1] [--install-dir PATH]

Environment variables:
  SENIX_VERSION       Release tag to install
  SENIX_INSTALL_DIR   Destination directory, default: /usr/local/bin
  SENIX_REPOSITORY    GitHub owner/repository, default: ALVIN-YANG/senix
  SENIX_RELEASE_BASE_URL  Trusted HTTPS release mirror base URL
EOF
}

download() {
  case "$release_base_url" in
    https://*)
      curl --fail --show-error --silent --location \
        --proto '=https' --proto-redir '=https' \
        --connect-timeout 15 --retry 4 --retry-delay 2 --retry-all-errors \
        "$@"
      ;;
    http://127.0.0.1:*|http://localhost:*)
      # Loopback HTTP is accepted only for the installer integration test.
      curl --fail --show-error --silent --location \
        --connect-timeout 2 --retry 4 --retry-delay 0 --retry-all-errors \
        "$@"
      ;;
    *)
      echo "SENIX_RELEASE_BASE_URL must use HTTPS" >&2
      return 2
      ;;
  esac
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -ge 2 ] || { echo "--version requires a value" >&2; exit 2; }
      version=$2
      shift 2
      ;;
    --install-dir)
      [ "$#" -ge 2 ] || { echo "--install-dir requires a value" >&2; exit 2; }
      install_dir=$2
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

command -v curl >/dev/null 2>&1 || { echo "curl is required" >&2; exit 1; }
command -v tar >/dev/null 2>&1 || { echo "tar is required" >&2; exit 1; }

case "$(uname -s)" in
  Linux) os=unknown-linux-gnu ;;
  Darwin) os=apple-darwin ;;
  *) echo "unsupported operating system: $(uname -s)" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
esac

if [ -z "$version" ]; then
  latest_url=$(download --head --output /dev/null --write-out '%{url_effective}' "${release_base_url}/latest")
  version=${latest_url##*/}
fi

case "$version" in
  v*) ;;
  *) version="v${version}" ;;
esac

target="${arch}-${os}"
asset="senix-${version}-${target}.tar.gz"
base_url="${release_base_url}/download/${version}"
temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/senix-install.XXXXXX")
trap 'rm -rf "$temporary_dir"' EXIT INT TERM

echo "Downloading Senix ${version} for ${target}..."
download "${base_url}/${asset}" --output "${temporary_dir}/${asset}"
download "${base_url}/checksums.txt" --output "${temporary_dir}/checksums.txt"

expected=$(awk -v asset="$asset" '$2 == asset || $2 == "./" asset { print $1 }' "${temporary_dir}/checksums.txt")
[ -n "$expected" ] || { echo "checksum for ${asset} is missing" >&2; exit 1; }

if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "${temporary_dir}/${asset}" | awk '{print $1}')
else
  actual=$(shasum -a 256 "${temporary_dir}/${asset}" | awk '{print $1}')
fi

[ "$actual" = "$expected" ] || { echo "checksum verification failed for ${asset}" >&2; exit 1; }

tar -xzf "${temporary_dir}/${asset}" -C "$temporary_dir"
binary="${temporary_dir}/senix-${version}-${target}/senixd"
[ -x "$binary" ] || { echo "release archive does not contain senixd" >&2; exit 1; }

if [ -d "$install_dir" ] && [ -w "$install_dir" ]; then
  install -m 0755 "$binary" "${install_dir}/senixd"
elif command -v sudo >/dev/null 2>&1; then
  sudo mkdir -p "$install_dir"
  sudo install -m 0755 "$binary" "${install_dir}/senixd"
else
  echo "${install_dir} is not writable; set SENIX_INSTALL_DIR to a writable directory" >&2
  exit 1
fi

echo "Installed ${install_dir}/senixd"
"${install_dir}/senixd" --version
