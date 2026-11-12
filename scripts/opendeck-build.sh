#!/bin/sh
set -eu
OPENDECK_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
case "$#:${1:-}" in
  0:|1:--build-only) ;;
  *) echo "Usage: $0 [--build-only]" >&2; exit 2 ;;
esac
export CARGO_HOME="$OPENDECK_ROOT/.build-cache/cargo"
"$OPENDECK_ROOT/scripts/build-lame-dylib.sh"
cd "$OPENDECK_ROOT/apps/compas-dj/frontend"
if [ ! -x node_modules/.bin/tauri ]; then
  npm ci --cache "$OPENDECK_ROOT/.build-cache/npm"
fi
cd ..
frontend/node_modules/.bin/tauri build --debug --bundles app
APP="$OPENDECK_ROOT/target/debug/bundle/macos/Open Deck.app"
if [ ! -x "$APP/Contents/MacOS/compas-dj" ]; then
  echo "tauri did not produce $APP" >&2
  exit 1
fi
mkdir -p "$APP/Contents/Frameworks" "$APP/Contents/Resources"
cp -f "$OPENDECK_ROOT/.build-cache/lame/lib/libmp3lame.0.dylib" "$APP/Contents/Frameworks/libmp3lame.0.dylib"
cp -f "$OPENDECK_ROOT/third_party/lame/COPYING" "$APP/Contents/Resources/LAME-COPYING.txt"
cp -f "$OPENDECK_ROOT/third_party/lame/LICENSE" "$APP/Contents/Resources/LAME-LICENSE.txt"
if [ "${1:-}" != "--build-only" ]; then
  exec "$OPENDECK_ROOT/scripts/opendeck-install.sh"
fi
