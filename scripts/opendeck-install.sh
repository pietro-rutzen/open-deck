#!/bin/sh
# Install a finished local build without exposing a partially copied bundle to Finder.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
src="$root/target/debug/bundle/macos/Open Deck.app"
dest="/Applications/Open Deck.app"
test -x "$src/Contents/MacOS/compas-dj"
stage=$(mktemp -d "${TMPDIR:-/tmp}/opendeck-install.XXXXXX")
trap 'rm -rf "$stage"' EXIT
/usr/bin/ditto "$src" "$stage/Open Deck.app"
/usr/bin/diff -qr "$src" "$stage/Open Deck.app"
python3 - "$stage/Open Deck.app" "$dest" <<'PY'
import ctypes
import os
import plistlib
import sys
from pathlib import Path

source, target = map(Path, sys.argv[1:])
def bundle_id(app):
    with (app / "Contents/Info.plist").open("rb") as plist:
        return plistlib.load(plist)["CFBundleIdentifier"]

if target.is_symlink():
    raise SystemExit("Refusing to replace a symlink in Applications")
if target.exists():
    if bundle_id(source) != bundle_id(target):
        raise SystemExit("The installed app has a different bundle identifier")
    # macOS renamex_np(RENAME_SWAP) exchanges both directories in one operation.
    libc = ctypes.CDLL(None, use_errno=True)
    swap = libc.renamex_np
    swap.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
    swap.restype = ctypes.c_int
    if swap(os.fsencode(source), os.fsencode(target), 2):
        err = ctypes.get_errno()
        raise OSError(err, os.strerror(err), str(target))
else:
    os.rename(source, target)
PY
/usr/bin/diff -qr "$src" "$dest"
printf '%s\n' "Installed and verified $dest"

# Quit only after installation succeeds so a failed build or swap leaves playback alone.
bundle_id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$dest/Contents/Info.plist")
/usr/bin/osascript -l JavaScript - "$bundle_id" <<'JXA'
ObjC.import('AppKit')
function run(argv) {
  const apps = $.NSRunningApplication.runningApplicationsWithBundleIdentifier(argv[0])
  const running = []
  for (let i = 0; i < apps.count; i++) {
    const app = apps.objectAtIndex(i)
    if (!app.terminated && !app.terminate && !app.terminated)
      throw Error('Open Deck declined to quit. Quit it manually, then reopen the installed app.')
    running.push(app)
  }
  const deadline = Date.now() + 15000
  while (running.some(app => !app.terminated)) {
    if (Date.now() >= deadline)
      throw Error('Open Deck is still closing. Quit it manually, then reopen the installed app.')
    $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(0.1))
  }
}
JXA
/usr/bin/open "$dest"
