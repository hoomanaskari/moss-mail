#!/bin/sh
# Sourced by app.sh and release.sh. Sets APPLE_SIGNING_IDENTITY, unless it's already set, to the
# Developer ID Application certificate on this Mac that lasts longest. With none, leaves it empty.

if [ -z "$APPLE_SIGNING_IDENTITY" ] && [ "$(uname)" = Darwin ]; then
  newest=0
  for hash in $(security find-identity -v -p codesigning | awk '/"Developer ID Application:/ { print $2 }'); do
    ends=$(security find-certificate -a -Z -p | awk -v h="$hash" '
      /^SHA-1 hash:/ { keep = ($3 == h) }
      keep' | openssl x509 -noout -enddate | cut -d= -f2)
    ends=$(date -j -f "%b %e %T %Y %Z" "$ends" +%s)
    if [ "$ends" -gt "$newest" ]; then
      newest=$ends
      APPLE_SIGNING_IDENTITY=$hash
    fi
  done
fi
