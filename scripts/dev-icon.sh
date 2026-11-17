#!/bin/sh
# Moss Dev's icon is Moss's with an orange DEV band, so the two never look alike in the Dock.
# This redraws src-tauri/icons/dev/ from src-tauri/icons/Moss.icon: the Icon Composer file
# (compiled into Moss Dev's Assets.car by `sh scripts/app-icon.sh --dev`), the Dock pictures for
# when Moss's own theme differs from the Mac's, and icon.icns with the PNGs Tauri wants.
# Run it after changing Moss.icon, and commit what it writes. Needs Xcode (Icon Composer).
set -e
[ "$(uname)" = Darwin ] || exit 0

root=$(cd "$(dirname "$0")/.." && pwd)
icons="$root/src-tauri/icons"
dev="$icons/dev"
ictool="$(xcode-select -p)/../Applications/Icon Composer.app/Contents/Executables/ictool"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' 0

mkdir -p "$dev"
rm -rf "$dev/Moss.icon"
cp -R "$icons/Moss.icon" "$dev/Moss.icon"
for look in light dark; do
  swift "$root/scripts/dev-icon.swift" "$icons/Moss.icon/Assets/$look.png" "$dev/Moss.icon/Assets/$look.png"
done

# The Dock pictures, drawn as `app-icon.sh --dock` draws Moss's: a 412 px tile in 512 px.
for look in light:Default dark:Dark; do
  name=${look%%:*}
  "$ictool" "$dev/Moss.icon" --export-image --output-file "$tmp/$name.png" --platform macOS \
    --rendition "${look#*:}" --width 412 --height 412 --scale 1 >/dev/null
  sips --padToHeightWidth 512 512 "$tmp/$name.png" --out "$dev/dock-$name.png" >/dev/null
done

# icon.icns and the PNGs, from an 824 px tile in 1024 px (the light look).
"$ictool" "$dev/Moss.icon" --export-image --output-file "$tmp/tile.png" --platform macOS \
  --rendition Default --width 824 --height 824 --scale 1 >/dev/null
sips --padToHeightWidth 1024 1024 "$tmp/tile.png" --out "$tmp/1024.png" >/dev/null
mkdir "$tmp/icon.iconset"
for size in 16 32 128 256 512; do
  sips -z $size $size "$tmp/1024.png" --out "$tmp/icon.iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z $double $double "$tmp/1024.png" --out "$tmp/icon.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$tmp/icon.iconset" -o "$dev/icon.icns"
sips -z 32 32 "$tmp/1024.png" --out "$dev/32x32.png" >/dev/null
sips -z 128 128 "$tmp/1024.png" --out "$dev/128x128.png" >/dev/null
sips -z 256 256 "$tmp/1024.png" --out "$dev/128x128@2x.png" >/dev/null
echo "Drew Moss Dev's icon in $dev"
