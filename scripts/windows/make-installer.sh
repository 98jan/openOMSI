#!/bin/sh
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)"
dist="${1:-dist/windows}"
out="${2:-out/openOMSI-setup.exe}"
version="${OPENOMSI_VERSION:-$(sh scripts/version.sh 2>/dev/null || echo 0.0.0)}"
mkdir -p "$(dirname "$out")"
list="$(cd "$(dirname "$out")" && { pwd -W 2>/dev/null || pwd; })/$(basename "${out%.exe}")-uninstall.nsh"
for f in "$dist"/*; do
  [ -f "$f" ] && printf 'Delete "$INSTDIR\\%s"\n' "${f##*/}"
done > "$list"
makensis -V2 -NOCD -DDIST="$dist" -DOUT="$out" -DVERSION="$version" -DUNINSTALL_LIST="$list" scripts/windows/installer.nsi
rm -f "$list"
