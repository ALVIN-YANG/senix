#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: $0 <target> <version> <output-directory>" >&2
  exit 2
fi

target=$1
version=$2
output_dir=$3
case "$version" in
  ''|*[!A-Za-z0-9._+-]*)
    echo "version must contain only letters, digits, dots, underscores, plus or minus signs" >&2
    exit 2
    ;;
esac
archive="senix-${version}-${target}"
doc_ref=${version#edge-}
stage_dir="${output_dir}/${archive}"

mkdir -p "$stage_dir/examples"
cp "target/${target}/release/senixd" "$stage_dir/senixd"
sed -e "s|@SENIX_VERSION@|$version|g" -e "s|@SENIX_DOC_REF@|$doc_ref|g" \
  docs/releases/archive-readme.md > "$stage_dir/README.md"
cp LICENSE "$stage_dir/"
cp examples/gateway.json "$stage_dir/examples/gateway.json"
chmod 0755 "$stage_dir/senixd"
tar -C "$output_dir" -czf "${output_dir}/${archive}.tar.gz" "$archive"
rm -r "$stage_dir"
