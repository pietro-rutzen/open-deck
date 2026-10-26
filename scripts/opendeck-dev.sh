#!/bin/sh
set -eu
OPENDECK_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export CARGO_HOME="$OPENDECK_ROOT/.build-cache/cargo"
cd "$OPENDECK_ROOT/apps/compas-dj/frontend"
if [ ! -x node_modules/.bin/tauri ]; then
  npm ci --cache "$OPENDECK_ROOT/.build-cache/npm"
fi
cd ..
exec frontend/node_modules/.bin/tauri dev
