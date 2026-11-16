#!/bin/sh
# Validation, packaging and draft publication are coordinated from one isolated snapshot.
set -e
exec node "$(dirname "$0")/release.ts" "$@"
