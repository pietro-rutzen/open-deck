OpenDeck loads unmodified LAME 3.100 as a shared library (`libmp3lame`) and does not
link it statically. LAME's own LICENSE says the straightforward way to ship it is as
a separate library. The build script downloads the upstream tarball, drops one obsolete
export so the shared library links, and installs the dylib under `.build-cache/lame`.
The app bundle copies that dylib into `Contents/Frameworks`, where it can be replaced,
and copies COPYING and LICENSE into `Contents/Resources`.

Build: `scripts/build-lame-dylib.sh`

Upstream: https://lame.sourceforge.io/ and https://www.mp3dev.org/
