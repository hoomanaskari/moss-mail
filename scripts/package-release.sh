#!/bin/sh
# Internal packaging phase. Run pnpm release so validation gates this script.
set -e
cd "$(dirname "$0")/.."

local=false
case "$1" in
  --local) local=true ;;
  --production) ;;
  *) echo "Use pnpm release [--local]" >&2; exit 1 ;;
esac

VERSION=$(node -e 'const v = require("./src-tauri/tauri.conf.json").version; if (typeof v !== "string" || !/^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$/.test(v)) throw Error("Invalid release version"); console.log(v)')
TAG="v$VERSION"
REPO=hoomanaskari/moss-mail
KEY="${MOSS_UPDATER_KEY:-$HOME/.tauri/moss-updater.key}"
PROFILE="${MOSS_NOTARY_PROFILE:-moss-notary}"
NOTES="docs/releases/$VERSION.md"
OUT="target/release/moss-release/$VERSION"
APP=target/release/bundle/macos/Moss.app
ARCHIVE="Moss_${VERSION}_aarch64.app.tar.gz"
DMG="Moss_${VERSION}_aarch64.dmg"

stop() {
  echo "Stopped: $1" >&2
  exit 1
}

# Check signing prerequisites before packaging.
[ "$(uname -m)" = arm64 ] || stop "build releases on an Apple silicon Mac."
[ -f "$KEY" ] || stop "no update signing key at $KEY. See docs/releasing.md."
[ -f "$NOTES" ] || stop "write what's new in $NOTES first."
command -v create-dmg >/dev/null || stop "create-dmg is missing. Install it with: brew install create-dmg"
. scripts/signing-identity.sh
[ -n "$APPLE_SIGNING_IDENTITY" ] || stop "no Developer ID Application certificate on this Mac."
export APPLE_SIGNING_IDENTITY
if ! $local; then
  xcrun notarytool history --keychain-profile "$PROFILE" >/dev/null 2>&1 || stop "no notary login saved as \"$PROFILE\". See docs/releasing.md."
fi

[ -f target/moss-release-source.json ] || stop "validated source provenance is missing. Use pnpm release."
[ ! -e "$OUT" ] || stop "package output already exists in this snapshot."
echo "Building Moss $VERSION, signed with $APPLE_SIGNING_IDENTITY"
pnpm tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false,"resources":{"../target/moss-release-source.json":"release-source.json"}}}' -- --locked
mkdir -p "$OUT"

notarize() {
  echo "Notarizing $(basename "$1") (Apple usually takes a few minutes)"
  xcrun notarytool submit "$1" --keychain-profile "$PROFILE" --wait
}

if $local; then
  codesign --verify --deep --strict "$APP"
else
  ditto -c -k --keepParent "$APP" "$OUT/Moss-for-notary.zip"
  notarize "$OUT/Moss-for-notary.zip"
  rm "$OUT/Moss-for-notary.zip"
  xcrun stapler staple "$APP"
  spctl --assess --type execute "$APP"
fi

echo "Making the update archive"
COPYFILE_DISABLE=1 tar -czf "$OUT/$ARCHIVE" -C "$(dirname "$APP")" Moss.app
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" pnpm -s tauri signer sign --app-version "$VERSION" -f "$KEY" "$OUT/$ARCHIVE" >/dev/null

echo "Making the DMG"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' 0
trap 'exit 130' INT
trap 'exit 143' TERM
ditto "$APP" "$STAGE/Moss.app"
create-dmg --volname "Moss $VERSION" --window-size 540 360 --icon-size 96 \
  --icon Moss.app 140 170 --hide-extension Moss.app --app-drop-link 400 170 \
  --codesign "$APPLE_SIGNING_IDENTITY" --no-internet-enable \
  "$OUT/$DMG" "$STAGE" >/dev/null
rm -rf "$STAGE"
if ! $local; then
  notarize "$OUT/$DMG"
  xcrun stapler staple "$OUT/$DMG"
fi

# What Moss's updater reads: the newest version, its notes, and where its signed archive is.
VERSION="$VERSION" NOTES="$NOTES" SIG="$OUT/$ARCHIVE.sig" \
  URL="https://github.com/$REPO/releases/download/$TAG/$ARCHIVE" node -e '
  const fs = require("fs");
  const e = process.env;
  const latest = {
    version: e.VERSION,
    notes: fs.readFileSync(e.NOTES, "utf8").trim(),
    pub_date: new Date().toISOString(),
    platforms: { "darwin-aarch64": { signature: fs.readFileSync(e.SIG, "utf8").trim(), url: e.URL } },
  };
  fs.writeFileSync(process.argv[1], JSON.stringify(latest, null, 2) + "\n");
' "$OUT/latest.json"

echo "Packaged Moss $VERSION in $OUT. The release coordinator verifies these files before upload."
