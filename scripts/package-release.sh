#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <target> <version> <output-directory>" >&2
  exit 2
fi

target=$1
version=$2
output_dir=$3
archive="senix-${version}-${target}"
stage_dir="${output_dir}/${archive}"

mkdir -p "$stage_dir/examples"
cp "target/${target}/release/senixd" "$stage_dir/senixd"
cp README.md LICENSE "$stage_dir/"
cp examples/gateway.json "$stage_dir/examples/gateway.json"
chmod 0755 "$stage_dir/senixd"
tar -C "$output_dir" -czf "${output_dir}/${archive}.tar.gz" "$archive"
rm -r "$stage_dir"
