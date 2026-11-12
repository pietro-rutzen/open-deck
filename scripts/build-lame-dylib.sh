#!/bin/sh
# Build unmodified LAME 3.100 as a shared library the app can dlopen.
# LGPL: the dylib is replaceable. Sources are downloaded, not vendored.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
PREFIX="$ROOT/.build-cache/lame"
DYLIB="$PREFIX/lib/libmp3lame.0.dylib"
# Match the compiler Cargo uses. A Rosetta shell reports x86_64 while rustc
# on this machine is aarch64, and dlopen rejects the other slice.
HOST=$(rustc -vV | sed -n 's/^host: //p')
case "$HOST" in
  aarch64-apple-darwin) ARCH=arm64; ARCH_CMD="arch -arm64" ;;
  x86_64-apple-darwin) ARCH=x86_64; ARCH_CMD="arch -x86_64" ;;
  *) echo "unsupported rustc host: $HOST" >&2; exit 1 ;;
esac
if [ -f "$DYLIB" ] && file "$DYLIB" | grep -q " $ARCH"; then
  exit 0
fi
rm -rf "$PREFIX"
SRC="$ROOT/.build-cache/lame-src"
mkdir -p "$SRC"
TARBALL="$SRC/lame-3.100.tar.gz"
if [ ! -f "$TARBALL" ]; then
  curl -L --fail --retry 3 -o "$TARBALL" \
    "https://sourceforge.net/projects/lame/files/lame/3.100/lame-3.100.tar.gz/download"
fi
rm -rf "$SRC/lame-3.100"
tar -xzf "$TARBALL" -C "$SRC"
cd "$SRC/lame-3.100"
# LAME 3.100 compiles the obsolete lame_init_old as a static function, but
# still lists it in the shared-library export file. Drop that one export so
# the link succeeds. The encoder itself is unchanged.
grep -v '^lame_init_old$' include/libmp3lame.sym > include/libmp3lame.sym.tmp
mv include/libmp3lame.sym.tmp include/libmp3lame.sym
# The 2017 sources call a few C functions without prototypes. Current Apple
# clang treats that as an error unless it is relaxed. The frontend is unused.
export CFLAGS="-O2 -Wno-implicit-function-declaration -Wno-deprecated-non-prototype"
$ARCH_CMD ./configure --prefix="$PREFIX" --enable-shared --disable-static --disable-frontend
$ARCH_CMD make -j"$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"
$ARCH_CMD make install
if [ ! -f "$DYLIB" ]; then
  found=$(find "$PREFIX" -name 'libmp3lame*.dylib' -type f | head -1)
  if [ -z "$found" ]; then
    echo "libmp3lame was not produced" >&2
    exit 1
  fi
  mkdir -p "$PREFIX/lib"
  cp "$found" "$DYLIB"
fi
install_name_tool -id "@rpath/libmp3lame.0.dylib" "$DYLIB"
test -f "$DYLIB"
