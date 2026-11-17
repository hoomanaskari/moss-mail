#!/bin/sh
# Builds Moss Dev on this Mac: target/release/bundle/macos/Moss Dev.app. It is a separate app
# from the installed Moss (its own name, identifier, data folder, Keychain items, agent port and
# Dock icon, and no updater), so trying a change never touches the mail you use. The installed
# Moss is made only by `pnpm release`.
#
# Signed with one identity that stays the same from build to build. The Keychain ties "Always
# Allow" to that signature: an ad-hoc build looks like a new app every time, so macOS would ask
# for your password again after each rebuild and each sign-in.
# Signs with APPLE_SIGNING_IDENTITY when set; otherwise with the Developer ID Application
# certificate on this Mac that lasts longest; with none, ad-hoc (and the Keychain will ask).
set -e

root=$(cd "$(dirname "$0")/.." && pwd)
. "$root/scripts/signing-identity.sh"

if [ -n "$APPLE_SIGNING_IDENTITY" ]; then
  export APPLE_SIGNING_IDENTITY
  echo "Signing Moss Dev.app with $APPLE_SIGNING_IDENTITY"
else
  echo "No Developer ID certificate: Moss Dev.app will be signed ad-hoc"
fi

# The overlay also turns off the update archive, which only `pnpm release` makes.
pnpm tauri build --bundles app --config "$root/src-tauri/tauri.dev.conf.json" "$@"
