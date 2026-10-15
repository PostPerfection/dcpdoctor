#!/usr/bin/env bash
set -euo pipefail

binary=$1
repository=$(cd "$(dirname "$0")/.." && pwd)
# the ST 429-9 schema rejects this element and the ASSETMAP parser ignores it
probe_element=SchemaPassProbe

package=$(mktemp -d)
cp -R "$repository/tests/fixtures/valid_smpte/." "$package"
perl -pi -e "s|<VolumeCount>|<$probe_element/><VolumeCount>|" "$package/ASSETMAP.xml"

# a binary built in this checkout falls back to these schemas
mv "$repository/schemas" "$repository/schemas.hidden"
restore() {
  mv "$repository/schemas.hidden" "$repository/schemas"
  rm -rf "$package"
}
trap restore EXIT

unset DCPDOCTOR_SCHEMA_DIR
output=$("$binary" validate "$package" 2>&1 || true)
if ! grep -q "xml_schema_violation.*$probe_element" <<<"$output"
then
  echo "$output"
  echo "$binary did not report the XSD violation it was given, so its XSD pass did not run" >&2
  exit 1
fi
grep "xml_schema_violation.*$probe_element" <<<"$output"
