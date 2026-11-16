#!/bin/sh
# Builds Moss.app on this Mac, signed with one identity that stays the same from build to build.
# The Keychain ties "Always Allow" to that signature: an ad-hoc build looks like a new app every
# time, so macOS would ask for your password again after each rebuild and each sign-in.
#
# Signs with APPLE_SIGNING_IDENTITY when set; otherwise with the Developer ID Application
# certificate on this Mac that lasts longest; with none, ad-hoc (and the Keychain will ask).
# Builds for trying on this Mac skip the update archive; `pnpm release` makes that.
set -e

. "$(dirname "$0")/signing-identity.sh"

if [ -n "$APPLE_SIGNING_IDENTITY" ]; then
  export APPLE_SIGNING_IDENTITY
  echo "Signing Moss.app with $APPLE_SIGNING_IDENTITY"
else
  echo "No Developer ID certificate: Moss.app will be signed ad-hoc"
fi

pnpm tauri build --bundles app --config '{"bundle":{"createUpdaterArtifacts":false}}' "$@"
