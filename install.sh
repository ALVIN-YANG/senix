#!/bin/sh
set -eu

repository=${SENIX_REPOSITORY:-ALVIN-YANG/senix}
version=${SENIX_VERSION:-}
install_dir=${SENIX_INSTALL_DIR:-/usr/local/bin}

usage() {
  cat <<'EOF'
Install the latest Senix release from GitHub.

Usage: install.sh [--version v0.2.0] [--install-dir PATH]

Environment variables:
  SENIX_VERSION       Release tag to install
  SENIX_INSTALL_DIR   Destination directory, default: /usr/local/bin
  SENIX_REPOSITORY    GitHub owner/repository, default: ALVIN-YANG/senix
EOF
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
  latest_url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/${repository}/releases/latest")
  version=${latest_url##*/}
fi

case "$version" in
  v*) ;;
  *) version="v${version}" ;;
esac

target="${arch}-${os}"
asset="senix-${version}-${target}.tar.gz"
base_url="https://github.com/${repository}/releases/download/${version}"
temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/senix-install.XXXXXX")
trap 'rm -rf "$temporary_dir"' EXIT INT TERM

echo "Downloading Senix ${version} for ${target}..."
curl -fsSL "${base_url}/${asset}" -o "${temporary_dir}/${asset}"
curl -fsSL "${base_url}/checksums.txt" -o "${temporary_dir}/checksums.txt"

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
