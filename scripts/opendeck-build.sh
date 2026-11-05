#!/bin/sh
set -eu
OPENDECK_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
case "$#:${1:-}" in
  0:|1:--build-only) ;;
  *) echo "Usage: $0 [--build-only]" >&2; exit 2 ;;
esac
export CARGO_HOME="$OPENDECK_ROOT/.build-cache/cargo"
cd "$OPENDECK_ROOT/apps/compas-dj/frontend"
if [ ! -x node_modules/.bin/tauri ]; then
  npm ci --cache "$OPENDECK_ROOT/.build-cache/npm"
fi
cd ..
frontend/node_modules/.bin/tauri build --debug --bundles app
if [ "${1:-}" != "--build-only" ]; then
  exec "$OPENDECK_ROOT/scripts/opendeck-install.sh"
fi
