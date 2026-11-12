# MP3 mirrors

A watched folder keeps an `mp3/` directory beside its lossless files so an older CDJ has something to read. The library toolbar's last control is a "Keep MP3 copies" toggle, the same kind of button as Listening mode. It follows the watched folder that is open, and it is disabled until exactly one watched folder is open. New folders start on. A folder saved before this choice, with no explicit on or off, also starts on. Turning the button off stores that choice, and that folder stays off. Turning it off stops the sync and does not delete the copies.

The encoder, the LGPL link, and why the stream is LAME 320 kbps CBR are in `docs/decisions/0006-cdj-mp3-mirrors.md`.

## Where files go

`Artist/Album/song.flac` becomes `mp3/Artist/Album/song.mp3`, under the watched folder, when that directory has no custom order. The tree inside `mp3/` mirrors the tree of the folder. A nested directory that is itself named `mp3` is still a source: `nested/mp3/song.flac` becomes `mp3/nested/mp3/song.mp3`.

The direct child named `mp3` (any capitalization) is the mirror. The scanner does not enter it, and the library list, search, and crates do not show tracks whose path is `{watched folder}/mp3/...`. A folder you watch that is itself named `mp3` is a normal library; only the child of a watched folder is reserved.

Existing `.mp3` sources are skipped. They are not copied and not re-encoded. If two convertible files would share one MP3 name (`song.flac` and `song.wav`), the first path in byte order owns that name.

## Order on a CDJ

Older CDJs list a folder by file name. Some also walk the FAT directory in the order the entries were created. The name is the reliable order, so a directory uses numbered names only when every convertible file in it already has an OpenDeck reorder prefix (`01 - …`, `001. …`, or `1) …`). Those files are sorted by that number. The MP3 name is `{index} - {Artist} - {Title}.mp3`, padded to the number of digits in that directory's convertible count: nine files are `1` through `9`, a hundred files are `001` through `100`. Artist and title come from the library. A missing side is omitted. If both are missing, the name uses the source stem with its order prefix removed. Two numbered names that would still collide gain ` (2)`.

A directory that is not fully numbered keeps the plain stem, `song.mp3`. The column sort in the library is not used, because clicking a header would rename the mirror. The first path in byte order still owns a stem that two files would share.

Copies are encoded in destination-name order. A reorder parks every affected MP3 in the same directory and renames those temporary names onto the final names in that same order. Each rename is atomic. On a FAT volume that sequence is what a player walking directory entries sees after a fresh write or a full rename. A player that sorts by name uses the prefix even when the stick was filled by a copy that did not preserve entry order. The source reorder itself pads to at least two digits; the MP3 prefix uses only as many digits as the count needs.

The ID3 title and artist are the tags from the source file (BPM and key still follow the library analysis). The numbered prefix is only in the file name.

## What the manifest remembers

The manifest is version 2. Each entry is keyed by the source file's device and inode when the inode is non-zero, otherwise by its path. A version 1 manifest, which was keyed by the MP3 path, still loads. The first sync that still matches a version 1 entry records the inode. After that, renaming the source during Reorder renames the MP3 and does not encode it again. An entry whose source path and inode are both gone is deleted only after a fresh check that the source is missing. A missing or unreadable manifest still deletes nothing.

## What is converted

Every non-MP3 file the library already knows how to decode: FLAC, WAV, AIFF, AIF, M4A, AAC, OGG, and OGA. The output is always 44.1 kHz, 16-bit-equivalent, joint stereo, 320 kbps CBR. A source at another rate is resampled with rubato's 256-tap windowed sinc (`SincFixedIn`, Blackman-Harris, 256× oversampling, cutoff 0.95). Sources wider than 16-bit, and float sources, are dithered with triangular noise of one 16-bit LSB. A 16-bit integer source is rounded to the nearest code.

## When a file is redone

Each mirror has `mp3/.opendeck-mp3-manifest.json`. An entry records the source path, the destination path, the source device and inode, the source length, and the source modification time. A file is encoded only when that stamp differs or the MP3 is missing. A reorder that keeps the same inode, length, and modification time is a rename. When the source file is gone, the MP3 is deleted only if the manifest says OpenDeck created it. A missing or unreadable manifest deletes nothing. Files in `mp3/` that are not in the manifest are left alone.

The MP3 is written to `song.mp3.partial` in the same directory, the ID3v2.3 tag is added there, the file is synced, and then it is renamed. A drive pulled during the write keeps the previous MP3 or no MP3, not a half file.

Tags copied into ID3v2.3 are title, artist, album, genre, comment, a JPEG cover capped at 800 px on the long edge, TBPM, and TKEY. If the library has already analysed the track, that BPM and key replace the tags from the file. The musical key name is used when it exists, otherwise the Camelot code.

## Worker

One background thread does the work, at macOS background QoS. It does not run on the audio callback, and starting it does not scan the library on the main thread. Artist and title for the file names are read once per pass, on that thread. It pauses while any deck is playing. If the one-minute load average per CPU is above 1.25, it waits up to five seconds and then encodes one file, so a machine that is always busy still finishes. Progress events are sent at most every 400 ms, plus the moment a folder goes idle. Writes inside `mp3/` do not cause another library import; the folder watcher already ignores that directory.

The toolbar button is painted "Keep MP3 copies" and uses the same on state as the other toolbar toggles. When no single watched folder is open, it is disabled and the hover title says to open one. A fixed-width count (`3/12`) sits inside the button and stays blank while that folder is idle, so the other toolbar controls do not move.
