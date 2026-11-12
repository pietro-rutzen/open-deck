# MP3 mirrors

A watched folder can keep an `mp3/` directory beside its lossless files so an older CDJ has something to read. The checkbox on that folder is off until it is turned on. Turning it off stops the sync and does not delete the copies.

The encoder, the LGPL link, and why the stream is LAME 320 kbps CBR are in `docs/decisions/0006-cdj-mp3-mirrors.md`.

## Where files go

`Artist/Album/song.flac` becomes `mp3/Artist/Album/song.mp3`, under the watched folder. The tree inside `mp3/` mirrors the tree of the folder. A nested directory that is itself named `mp3` is still a source: `nested/mp3/song.flac` becomes `mp3/nested/mp3/song.mp3`.

The direct child named `mp3` (any capitalization) is the mirror. The scanner does not enter it, and the library list, search, and crates do not show tracks whose path is `{watched folder}/mp3/...`. A folder you watch that is itself named `mp3` is a normal library; only the child of a watched folder is reserved.

Existing `.mp3` sources are skipped. They are not copied and not re-encoded. If two convertible files would share one MP3 name (`song.flac` and `song.wav`), the first path in byte order owns that name.

## What is converted

Every non-MP3 file the library already knows how to decode: FLAC, WAV, AIFF, AIF, M4A, AAC, OGG, and OGA. The output is always 44.1 kHz, 16-bit-equivalent, joint stereo, 320 kbps CBR. A source at another rate is resampled with rubato's 256-tap windowed sinc (`SincFixedIn`, Blackman-Harris, 256× oversampling, cutoff 0.95). Sources wider than 16-bit, and float sources, are dithered with triangular noise of one 16-bit LSB. A 16-bit integer source is rounded to the nearest code.

## When a file is redone

Each mirror has `mp3/.opendeck-mp3-manifest.json`. An entry records the source path, the source length, and the source modification time. A file is encoded only when that stamp differs or the MP3 is missing. When the source file is gone, the MP3 is deleted only if the manifest says OpenDeck created it. A missing or unreadable manifest deletes nothing. Files in `mp3/` that are not in the manifest are left alone.

The MP3 is written to `song.mp3.partial` in the same directory, the ID3v2.3 tag is added there, the file is synced, and then it is renamed. A drive pulled during the write keeps the previous MP3 or no MP3, not a half file.

Tags copied into ID3v2.3 are title, artist, album, genre, comment, a JPEG cover capped at 800 px on the long edge, TBPM, and TKEY. If the library has already analysed the track, that BPM and key replace the tags from the file. The musical key name is used when it exists, otherwise the Camelot code.

## Worker

One background thread does the work, at macOS background QoS. It does not run on the audio callback. It pauses while any deck is playing. If the one-minute load average per CPU is above 1.25, it waits up to five seconds and then encodes one file, so a machine that is always busy still finishes. Writes inside `mp3/` do not cause another library import; the folder watcher already ignores that directory.

The folder row reserves a fixed-width count (`3/12`) so the number does not move the other controls. The count is blank when the folder is idle. The checkbox is a square with no corner radius. Its name, "Keep MP3 copies", is the hover title and the accessible name, not a painted label.
