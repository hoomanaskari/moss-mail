#!/bin/sh
# Moss's app icon has a light and a dark look, which macOS 26 and later switch with the Mac's
# appearance. Both live in src-tauri/icons/Moss.icon (open it in Icon Composer to change them).
#
#   sh scripts/app-icon.sh          compiles Moss.icon into src-tauri/gen/icon/Assets.car, which
#                                   the bundle copies into Moss.app (Tauri runs this before it
#                                   bundles). Info.plist names it with CFBundleIconName; older
#                                   macOS keeps using icons/icon.icns.
#   sh scripts/app-icon.sh --dock   also redraws icons/dock-light.png and icons/dock-dark.png, the
#                                   Dock icons Moss shows when its own theme differs from the
#                                   Mac's. Run it after changing Moss.icon, and commit them.
#   sh scripts/app-icon.sh --dev    compiles Moss Dev's icon, icons/dev/Moss.icon (drawn by
#                                   scripts/dev-icon.sh), into src-tauri/gen/icon-dev/Assets.car
#                                   instead: tauri.dev.conf.json runs this before it bundles.
set -e
[ "$(uname)" = Darwin ] || exit 0

root=$(cd "$(dirname "$0")/.." && pwd)
icons="$root/src-tauri/icons"
out="$root/src-tauri/gen/icon"
source="$icons/Moss.icon"
if [ "$1" = --dev ]; then
  source="$icons/dev/Moss.icon"
  out="$root/src-tauri/gen/icon-dev"
fi
mkdir -p "$out"

xcrun actool "$source" --compile "$out" --platform macosx \
  --minimum-deployment-target 11.0 --app-icon Moss \
  --output-partial-info-plist "$out/partial.plist" --errors --warnings >/dev/null

if [ "$1" = --dock ]; then
  ictool="$(xcode-select -p)/../Applications/Icon Composer.app/Contents/Executables/ictool"
  tmp=$(mktemp -d)
  for look in light:Default dark:Dark; do
    name=${look%%:*}
    # A Mac icon is an 824-point tile in the middle of a 1024-point canvas; drawn at 512 px,
    # that is 412 px with 50 px around it.
    "$ictool" "$icons/Moss.icon" --export-image --output-file "$tmp/$name.png" --platform macOS \
      --rendition "${look#*:}" --width 412 --height 412 --scale 1 >/dev/null
    sips --padToHeightWidth 512 512 "$tmp/$name.png" --out "$icons/dock-$name.png" >/dev/null
  done
  rm -rf "$tmp"
fi
