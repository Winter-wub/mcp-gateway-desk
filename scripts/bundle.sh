#!/usr/bin/env bash
# Build "MCP Gateway Desk.app" and a .dmg for it.
#
#   scripts/bundle.sh [--universal] <version>
#
# --universal builds Apple Silicon + Intel and joins them (needs both Rust targets).
# Signs ad hoc unless SIGN_IDENTITY is set (e.g. "Developer ID Application: …").
# Output: dist/MCP Gateway Desk.app, dist/MCP-Gateway-Desk-<version>.dmg (+ .sha256)
set -euo pipefail

UNIVERSAL=0
if [[ "${1:-}" == "--universal" ]]; then UNIVERSAL=1; shift; fi
VERSION="${1:?usage: scripts/bundle.sh [--universal] <version>}"
VERSION="${VERSION#v}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
BIN_NAME="mcp-gateway-desk"
APP_NAME="MCP Gateway Desk"
DIST="$ROOT/dist"
APP="$DIST/$APP_NAME.app"
DMG="$DIST/MCP-Gateway-Desk-$VERSION.dmg"

echo "==> Building $BIN_NAME $VERSION"
if [[ $UNIVERSAL == 1 ]]; then
  cargo build --release --locked --target aarch64-apple-darwin
  cargo build --release --locked --target x86_64-apple-darwin
  mkdir -p target/universal
  lipo -create -output "target/universal/$BIN_NAME" \
    "target/aarch64-apple-darwin/release/$BIN_NAME" \
    "target/x86_64-apple-darwin/release/$BIN_NAME"
  BIN="target/universal/$BIN_NAME"
else
  cargo build --release --locked
  BIN="target/release/$BIN_NAME"
fi

echo "==> Making $APP_NAME.app"
rm -rf "$APP" "$DMG" "$DMG.sha256"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/$BIN_NAME"
sed "s/__VERSION__/$VERSION/g" packaging/Info.plist > "$APP/Contents/Info.plist"

# Icon: drawn by the app itself, at every size macOS wants.
ICONSET="$(mktemp -d)/AppIcon.iconset"
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
  "$BIN" --write-icon "$ICONSET/icon_${s}x${s}.png" "$s"
  "$BIN" --write-icon "$ICONSET/icon_${s}x${s}@2x.png" "$((s * 2))"
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"

echo "==> Signing (${SIGN_IDENTITY:-ad hoc})"
codesign --force --deep --options runtime --timestamp=none -s "${SIGN_IDENTITY:--}" "$APP"
codesign --verify --deep --strict "$APP"

echo "==> Making $(basename "$DMG")"
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$STAGE"
(cd "$DIST" && shasum -a 256 "$(basename "$DMG")" > "$(basename "$DMG").sha256")
echo "==> Done"
cat "$DMG.sha256"
