#!/usr/bin/env bash
# Set the version and sha256 in Casks/mcp-gateway-desk.rb.
#   scripts/update-cask.sh <version> <sha256>
set -euo pipefail
VERSION="${1:?version}"; SHA="${2:?sha256}"
CASK="$(cd "$(dirname "$0")/.." && pwd)/Casks/mcp-gateway-desk.rb"
sed -i.bak -E "s/^  version \".*\"/  version \"${VERSION#v}\"/; s/^  sha256 \".*\"/  sha256 \"$SHA\"/" "$CASK"
rm -f "$CASK.bak"
grep -E '^  (version|sha256) ' "$CASK"
