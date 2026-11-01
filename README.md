# MusicWasp

A native music manager and player for **Linux and Windows**.

Website: <https://giwi.github.io/music-wasp/>

Point it at your music folders, index the tags, browse by genre / artist / album, play, fix bad
metadata, and keep playlists. No Electron, no web view, one native binary.

![MusicWasp](docs/screenshot.png)

## Status

This is an early but working project, not a finished product. What runs today:

| Area | Implemented |
| :-- | :-- |
| Library | incremental background scan of 18 audio formats, skips unchanged files by mtime + size, batches writes, tag reading via `lofty`, SQLite index |
| Browsing | column browser (genre → artist → album) with foldable sections whose state is remembered, advanced field search, sortable table |
| Track list | persisted column widths, drag/reorder/hide columns, and standard / compact / album-art row layouts |
| Languages | English (default), French, German and Spanish UI; switch live under Settings → General |
| Saved searches | name and reuse queries such as `artist:"radiohead" year:1997`, edit or delete them from the bookmark menu |
| Selection | multi-select (`Ctrl`-click, `Shift`-click, `Ctrl+A`, `Esc`), drag tracks onto a playlist, right-click a row for play / rename / metadata / playlist actions |
| Playback | play / pause / stop / next / prev / seek / volume, repeat (off / all / one), shuffle, gapless preloading and 1–12 s crossfade |
| Output | system default or any enumerated output device, switchable without restarting the app |
| ReplayGain | applies `REPLAYGAIN_TRACK_GAIN` / `REPLAYGAIN_ALBUM_GAIN` per source; choose album gain in Settings |
| Play queue | right-hand Up Next panel: current track, click any entry to jump, drag to reorder, right-click to remove, or clear everything after the current track; **Play next** / **Add to queue** from the track menu |
| Now playing | album art from embedded cover or folder sidecar, accent-highlighted current row, live position |
| Cover tools | album grid view with artwork, fetch missing covers from the Cover Art Archive, replace a cover from an image file, and save it as a `cover.jpg` sidecar |
| Tag editing | edit title, artist, album, album artist, genre, year, track no, disc no and write them into the file; batch edit writes only the fields you fill in |
| Tag batch actions | auto-number selected tracks per disc, normalize text case, clear one field, or copy a field from the first selected track; every action is undoable |
| Plain view | one-click full-window now-playing view: large cover, transport, seek, volume and the next track |
| Renaming | `F2` renames the clicked track field (title, artist, album, genre, year, track no) or the open playlist / smart playlist |
| Folder selection | in-app folder chooser (up/home, hidden folders, recursive scan) plus a path field |
| Auto-tagging | search MusicBrainz by artist + album; preview a file-to-release-track table with per-row overrides and tag diffs; match by disc/track/title/duration, write title/artist/album/album artist/year/genre/track/disc and embed the front cover; every write keeps a pre-write snapshot for Undo |
| File drops | drop audio files or folders from the OS file manager onto a playlist, **All music**, or an open playlist’s track list to import and add them |
| Playlists | save the current view or selection as a playlist, reorder by drag or Up/Down, remove tracks, drag rows onto another playlist to move them |
| Playlist folders | group playlists in nested folders; create, rename, delete, drag a playlist into a folder, or use the context menu |
| M3U import / export | import `.m3u` / `.m3u8` files (relative paths resolved, missing audio skipped, new files indexed) and export any playlist with `#EXTINF` metadata |
| Smart playlists | editable rule sets (field / operator / value, match all or any, sort, limit), re-evaluated from the database every time they open |
| Equalizer | 10-band peaking EQ + preamp, live curve, 8 presets, bypass; applies without restarting the track |
| Skins | nine built-in palettes plus user skins from `skins.json`, with per-skin colours, glass backdrops, labels and logo/icon; picked from Settings and remembered |
| Accessibility | logical Tab order, screen-reader labels for icon controls, larger text mode, high-contrast palette and visible focus rings |
| Settings | tabbed panel: General (skins, accessibility, folder watching), Keybindings, About; shortcuts and track-list column settings persist to `settings.json` |
| Library hygiene | find rows whose files vanished, detect duplicate artist/title/album groups, and remove entries from the library in bulk; audio files are never deleted |
| Folder watching | scanned folders are watched with `notify`: new audio is imported automatically and external tag edits refresh existing rows; manual **Scan** still works |
| Linux desktop | MPRIS2 service: desktop media keys, GNOME/KDE controls, lock screen, `playerctl` and Bluetooth headset buttons; publishes track, position, volume, shuffle and repeat |
| Scrobbling | optional ListenBrainz now-playing and completed-listen submissions; one toggle plus a user token, no developer API key |
| Session restore | reopens the last view, search, selection, volume, play queue and window size/position; playback always starts stopped |
| Portability | `MUSICWASP_DATA_DIR` for a portable library, data lives in the OS app-data dir by default |

Not there yet (see [Roadmap](#roadmap)): device sync, podcasts/radio, AcoustID fingerprinting.

## Build

Needs a stable Rust toolchain (`rustup`), version 1.85 or newer.

```bash
# Linux (Debian/Ubuntu)
sudo apt install libasound2-dev pkg-config
# Fedora
sudo dnf install alsa-lib-devel
# Arch
sudo pacman -S alsa-lib

cargo build --release
```

If you cannot install packages (no root, locked-down machine), run this once:

```bash
./scripts/dev-env.sh
```

It downloads the distro's ALSA dev package into `.sysroot/` without root.
`.cargo/config.toml` and `core/build.rs` then pick that directory up automatically, so plain
`cargo build` / `cargo run` work afterwards. On a normal machine, or on Windows, both are inert.

On **Windows** nothing extra is required beyond the MSVC toolchain that rustup installs
(SQLite is compiled in through `rusqlite`'s bundled feature, audio goes through WASAPI).

Run it:

```bash
cargo run --release -p musicwasp-gui     # GUI
cargo run --release -p musicwasp-cli -- stats
```

Release binaries land in `target/release/` as `musicwasp` (GUI) and `musicwasp-cli` (CLI).

### Desktop launcher (Linux)

To add MusicWasp to the desktop application menu with its own icon, build the
release binary and install a launcher entry:

```bash
cargo build --release
./scripts/install-desktop.sh
```

The entry lives in `packaging/musicwasp.desktop` and is installed to
`~/.local/share/applications/`, with `assets/ic_launcher-web.png` copied into the
hicolor icon theme. Pass a binary path to override which executable it runs.
The window itself keeps the MusicWasp logo as its window icon; only the launcher
entry uses the app icon.

Tagged Linux archives already contain the binaries, `packaging/musicwasp.desktop`,
`assets/ic_launcher-web.png` and `scripts/install-desktop.sh`; extract the archive and run
`./scripts/install-desktop.sh` (it finds the adjacent `musicwasp` binary). Windows builds embed
`assets/musicwasp.ico` into `musicwasp.exe`, so Explorer, shortcuts and the taskbar show the app
icon before the app starts.

## Usage

1. Start the GUI, open **Settings → General**, choose a folder with the folder button (or type a path),
   press **Scan**. Indexing runs in a background thread with live counts, batches its SQLite writes and can be
   stopped with the X next to the spinner. Unchanged files are skipped using mtime and size, so repeat scans on a
   large library only read new or edited files.
2. Browse with the left columns (click an entry again to clear it). Click a section caret to fold it;
   the folded state is saved like the rest of the settings. Type in **Search** to filter across
   title, artist, album and genre. Field filters narrow it further, for example
   `artist:"radiohead" year:1997`, `genre:rock year:>=1990` or `year:1990..1999`; text fields
   accept `=` for an exact match. Use the bookmark button beside the search box to save the current
   query, apply a saved one, or right-click a saved entry to edit or delete it. The **main menu**
   (☰) opens the column manager: drag table dividers to resize, reorder or hide columns, reset them,
   and choose the Standard, Compact or Album art layout. Widths, order, visibility and layout persist
   in `settings.json`.
3. Double-click a track, or select it and press `Enter`, to play. `Space` toggles pause. The
   **QUEUE** panel on the right shows the current track and **Up Next**: click an entry to jump to
   it, drag rows to reorder, right-click a row to remove it, or **Clear** everything after the
   current track. Toggle the panel from the main menu (☰) → **Play queue**.
4. Select tracks with `Ctrl`-click / `Shift`-click / `Ctrl+A`. Drag a selection onto a playlist in
   the sidebar to add it there, or right-click a track and use **Add to playlist**.
5. Right-click any track for **Play now**, **Play next**, **Add to queue**, **Rename**, **Edit tags**,
   **Find metadata**, **Remove from playlist** and **Add to playlist**. You can also drop audio files from your file manager onto a playlist,
   **All music**, or anywhere in an open playlist's track list to import and add them.
6. Select a whole album and press **Auto-tag**: search MusicBrainz and pick the right pressing.
   Before anything is written, review the **file-to-release-track** table: each row shows the local
   file, the suggested release track and the tag changes. Override any row with the track dropdown,
   or pick **skip** to leave that file untouched. Matching scores disc/track numbers, titles and
   durations before falling back to file order. **Apply to N track(s)** writes only the mapped rows
   and embeds the front cover by default. Before writing, the previous tags and embedded pictures
   are saved; use **Undo last tag write** in the dialog or main menu (☰) → **Undo last tag write**
   to restore them. The same undo snapshot is kept for editor and `F2` writes.
7. Select one track and press **Edit tags** (`Ctrl+F2`) to fix metadata; saved straight into the file.
   Select several and the editor becomes a batch editor: shared values are pre-filled, empty
   fields are left untouched in every file. The **Batch actions** section writes immediately:
   auto-number tracks (per disc), normalize case (title/lower/UPPER), clear a chosen field, or copy
   a field from the first selected track. All batch actions are covered by the same Undo snapshot.
8. The plus menu next to **PLAYLISTS** saves the current selection (or the whole view) as a
   playlist, creates an empty playlist, creates a playlist folder, or imports an M3U/M3U8 file.
   The new-playlist dialog also has a **Create empty playlist** checkbox. The sparkle button
   creates a smart playlist from rules. Right-click a playlist for
   **Rename**, **Export M3U…**, **Move to folder** and **Delete**; drag a playlist onto a folder to
   move it. Right-click a folder to create a playlist directly inside it, create a subfolder, rename
   it or delete it (its contents move up one level). Right-click a smart playlist and choose
   **Edit rules…** to change its rules, sort, limit or name. Inside a playlist, drag rows to reorder,
   use **Remove**/`Delete` to drop tracks, **Up**/**Down** (or `Alt`+arrows) to nudge a selection,
   and drag rows onto another playlist to move them there. Dropping M3U files from the file manager
   imports them too.
9. The main menu (☰) → **Plain now-playing view** switches to a plain now-playing view with large
   cover, transport, seek, volume and the next track. Choose it again to return to the library.
10. Click a track cell (title, artist, album, genre, year, track no) and press `F2` to rename that
   field in place. Click a playlist first and `F2` renames the playlist itself.
11. **Settings** (`Ctrl+,`) has three tabs: **General** for library folders, scanning, language,
    skins and folder watching, **Keybindings** for shortcuts, and **About** with the project link
    and copyright. In **Keybindings**, click a binding and press the new combination; `Delete`
    clears it, `Escape` cancels, and conflicts are shown in red.
12. **EQ** (`Ctrl+E`) opens the 10-band equalizer. Pick a preset or move the sliders; the curve and
   the running track update immediately. Changes are saved to `settings.json` automatically.
13. Pick a **Language** and a **Skin** in **Settings**. Languages are English (default), French,
    German and Spanish. Skins are MusicWasp Dark (default), Midnight, Forest, Mono, Winamp Classic,
    High contrast, Liquid Glass Dark, Liquid Glass White, or MusicWasp Light. Changes update
    instantly and are remembered. The General tab also toggles
    ReplayGain (with album-gain preference), picks the output device, sets the crossfade length
    (0 s keeps gapless joins) and enables **Larger text and controls** (a 120% interface scale).
    Tab and Shift+Tab follow the logical toolbar and panel order; icon-only buttons expose their
    tooltip text to screen readers.
14. Close and reopen the app: it comes back to the same view, search, selection, volume, queue and
    window size/position. Playback always starts stopped, with no track loaded; press `Space` to
    play the current queue entry from the beginning. Session state is written every few seconds and
    again on exit.
15. Main menu (☰) → **Library hygiene** lists rows whose files are gone and groups tracks that share
    artist/title/album. Tick rows (or use **Select all** / **Select all but the first in each group**)
    and confirm **Remove from library**. Only database entries are deleted: audio files stay on disk,
    and playlist entries disappear with the removed rows. Files can be re-imported at any time.
16. A successful background **Scan** also watches that folder. New audio files are imported
    automatically and tag changes made in another editor refresh their library rows. Toggle watching
    in **Settings**. Renames and deletions are not auto-pruned; **Library hygiene** reports those
    rows.
17. On Linux, MusicWasp registers `org.mpris.MediaPlayer2.musicwasp` with the session bus. Desktop
    media keys, GNOME/KDE media controls, the lock screen, `playerctl` and Bluetooth headset
    buttons control playback; track, position, volume, shuffle and repeat are published. Try
    `playerctl --player=musicwasp play-pause`.
18. Main menu (☰) → **Album grid** switches the track table to an **album grid**. Click a tile to
    open that album, or right-click it for **Fetch missing cover** (Cover Art Archive), **Replace
    cover…** (image picker, embedded into every track in the album) and **Save cover.jpg** beside
    the files. Sidecar covers are used automatically when a file has no embedded art.
19. **Scrobbling** uses ListenBrainz: enable it in **Settings → General** and paste a user token
    from your ListenBrainz profile. MusicWasp sends *playing now* when a track starts and a single
    listen after 50% or four minutes. No developer API key or Last.fm app registration is needed.

### Default shortcuts

| Action | Keys |
| :-- | :-- |
| Play / pause | `Space` |
| Play selected track | `Enter` |
| Next / previous track | `Ctrl+Right` / `Ctrl+Left` |
| Forward / backward 5 seconds | `Ctrl+Shift+Right` / `Ctrl+Shift+Left` |
| Stop | `Ctrl+.` |
| Volume up / down | `Ctrl+Up` / `Ctrl+Down` |
| Rename focused field / playlist | `F2` |
| Edit tags | `Ctrl+F2` |
| Focus search box | `Ctrl+F` |
| Open settings | `Ctrl+,` |
| Open equalizer | `Ctrl+E` |

Bindings live in `settings.json` beside the library database (`~/.local/share/musicwasp/` on
Linux, `%APPDATA%\musicwasp\` on Windows), and are written atomically on every change. Delete
the file, or press **Reset to defaults**, to start over.

### Custom skins

The built-in skin list, colours and logo choice live in
[`assets/skins.json`](assets/skins.json). To add or override skins, put a
`skins.json` beside `settings.json` in the data directory. Entries with the same `id` replace a
built-in skin; new ids are appended to the dropdown.

```json
{
  "skins": [
    {
      "id": "berry",
      "label": "Berry",
      "aliases": ["berry-pink"],
      "icon": "berry.png",
      "light": true,
      "glass": false,
      "palette": {
        "bg_dark": "#fbeef4",
        "bg_panel": "#fff7fa",
        "bg_row_alt": "#f8e4ed",
        "bg_selected": "#ffd6e7",
        "accent": "#d4145a",
        "text_dim": "#7a5262",
        "text": "#2a111b",
        "extreme_bg": "#f2dce6",
        "widget_inactive": "#f4dbe6",
        "widget_weak": "#f9ecf1",
        "widget_hover": "#efd1df"
      }
    }
  ]
}
```

Colours accept `#rrggbb` or `#rrggbbaa`. `icon` accepts `light`, `dark`, or a path relative to
`skins.json`. Missing palette fields fall back to MusicWasp Dark. `glass: true` enables the
translucent backdrop; an optional `backdrop` array supplies its four corner colours.

### Languages

English is the source language and default. French, German and Spanish catalogues live in
[`assets/i18n`](assets/i18n), with English source strings as keys and graceful fallback for any
missing entry. The language can be changed live in **Settings → General** and is persisted in
`settings.json`.

![Auto-tag by album](docs/autotag.png)

Editing a playlist (reorder, remove, drag between playlists):

![Playlist view](docs/playlist-view.png)

Smart playlists are rules, evaluated live:

![Smart playlist](docs/smart-playlist.png)

![Settings](docs/settings.png)

Finding missing files and duplicate rows without touching the audio:

![Library hygiene](docs/library-hygiene.png)

Album grid with cover tools:

![Album grid](docs/album-grid.png)

Batch editing several tracks at once:

![Batch edit](docs/batch-edit.png)

Choosing the folder to scan:

![Folder picker](docs/folder-picker.png)

Roughly 5k tracks index in a couple of seconds; the table stays responsive while playing because
audio runs on its own thread and the UI reads lock-free snapshots.

CLI, handy for scripting and for checking the library without a display:

```bash
musicwasp-cli scan ~/Music          # index a folder
musicwasp-cli scan ~/Music --rescan # re-read tags even for unchanged files
musicwasp-cli list --filter "floyd"
musicwasp-cli list --filter 'artist:radiohead year:1997'
musicwasp-cli play 42               # play library track 42
musicwasp-cli stats
musicwasp-cli playlist list
musicwasp-cli playlist import ~/Music/mix.m3u8 --folder Roadtrip
musicwasp-cli playlist export 3 ~/Music/roadtrip.m3u8
musicwasp-cli playlist folder-create "Roadtrip"

# auto-tag by album: search, then apply the best match to tracks 1-12
musicwasp-cli autotag -r "Radiohead" -l "OK Computer"
musicwasp-cli autotag -r "Radiohead" -l "OK Computer" \
    --apply best --ids 1,2,3,4,5,6,7,8,9,10,11,12 --cover
```

Auto-tagging talks to `musicbrainz.org` and `coverartarchive.org`: no API key, but it needs
network access and respects MusicBrainz's one-request-per-second limit (with retry/backoff on
429/503). AcoustID is **not** wired up: it needs a registered API key *and* a local
`fpcalc`/chromaprint install to fingerprint files, so this path uses release lookup instead —
which is what "tag by album" needs in practice.

## Architecture

```
core/   musicwasp-core   library DB (SQLite), tags, playback/DSP, auto-tag, playlists
gui/    musicwasp        egui/eframe UI, skins, settings, playlists
cli/    musicwasp-cli    headless library tool
```

- **`core::library`** owns the connection. `Track`/`Filter`/`SortKey` are the only types the UI
  needs, so SQL stays in one place. Scans compare mtime and size against a path map and commit in
  batches; the GUI runs them on a worker with its own connection and a small progress channel.
- **`core::player`** spawns an audio thread that owns the `cpal` output stream and a `rodio` sink.
  The UI sends commands over an `mpsc` channel and reads an `Arc<Shared>` of atomics, so a slow
  or missing audio device can never freeze the interface. Decoded samples pass through
  **`core::dsp::DspSource`** before reaching the sink; the shared [`EqShared`] handle lets the UI
  change the equalizer while a track is playing.
- **`core::dsp`** is a 10-band RBJ peaking EQ with a preamp. Filters are biquads, one chain per
  channel, and settings arrive as a snapshot with a version counter so the audio callback does not
  take a mutex for every sample. Bypass is a straight pass-through; output is clamped to avoid wrap
  clipping at extreme settings.
- **`core::tags`** writes through `lofty`, then the row is re-read so the UI never shows stale data.
- **`core::playlist`** is a plain join table; playlist order is explicit, and move/remove keep
  positions dense so reordering is a single ordered rewrite. Playlist folders are a small
  self-referencing tree; deleting a folder promotes its children instead of hiding them.
- **`core::m3u`** parses `#EXTINF` metadata, resolves relative and `file://` paths, imports missing
  audio through the normal tag reader, and exports absolute paths with `#EXTINF` for other players.
- **`core::search`** parses field filters into a small typed query, compiles them to parameterised
  SQL, and also stores named queries so the GUI and CLI share the same syntax.
- **`core::smart`** stores a JSON rule set and compiles it to one parameterised SELECT, with the
  field/operator names coming from enums rather than user text.
- **`core::autotag`** is split into pure steps (`search`, `fetch`, `suggest`, `apply`) so matching
  and tag writing are unit-tested without network; only `search`/`fetch`/`front_cover` do I/O, from
  a worker thread so the UI never blocks.
- **`gui::settings`** maps actions to a normalised token (`ctrl+right`, `space`), stores them as
  JSON, and reports conflicts; the app dispatches keys from that table instead of hardcoding them.
  It also persists the equalizer state, selected skin and track-list column configuration.
- **`gui::theme`** is a small skin engine: named palettes drive egui's `Visuals` and the colours
  that custom-painted widgets (equalizer curve, drag marker, album-art placeholder) use directly.
  It also owns the high-contrast palette, visible focus strokes, the larger-text UI scale and the
  translucent Liquid Glass gradients. Skin ids, labels, colours, backdrops and icons come from
  `assets/skins.json`, with an optional user override in the data directory.

Database schema (SQLite, `PRAGMA journal_mode=WAL`):

```
tracks(id, path UNIQUE, title, artist, album, album_artist, genre, year,
       track_no, disc_no, duration_secs, sample_rate, bitrate, channels,
       format, size_bytes, mtime, added_at)
playlists(id, name, folder_id)
playlist_folders(id, name, parent_id)
playlist_tracks(playlist_id, track_id, position)
saved_searches(id, name, query)
```

## Testing

```bash
cargo test --workspace          # core, CLI and headless GUI regression tests
cargo test -p musicwasp-gui ui_tests
```

The tests synthesise their own WAV files, so they need no sample library and no audio device.
GUI tests use `egui_kittest` to drive the real app through AccessKit, including search, column
layout changes and the auto-tag dialog. Auto-tag tests point the MusicBrainz and Cover Art Archive
base URLs at an in-process fake HTTP server, so they never touch the network.

For a live development loop, install `cargo-watch` and run:

```bash
cargo install cargo-watch --locked
scripts/dev-watch.sh                   # launch and restart the GUI on every save
WATCH_MODE=check scripts/dev-watch.sh  # fast cargo check instead
WATCH_MODE=test scripts/dev-watch.sh   # full tests instead
```

Dev-only screenshot hooks (used to produce the images above):

```bash
MUSICWASP_SCREENSHOT=/tmp/shot.png MUSICWASP_AUTOPLAY=1 cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/plain.png MUSICWASP_AUTOPLAY=1 MUSICWASP_PLAIN=1 cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/tags.png MUSICWASP_EDITOR=1 cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/rename.png MUSICWASP_RENAME=1 cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/eq.png MUSICWASP_EQ=1 cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/columns.png MUSICWASP_COLUMNS=1 cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/scan.png MUSICWASP_SCAN=/path/to/music cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/settings.png MUSICWASP_SETTINGS=1 MUSICWASP_SETTINGS_TAB=about cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/hygiene.png MUSICWASP_HYGIENE=1 cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/grid.png MUSICWASP_ALBUM_GRID=1 cargo run -p musicwasp-gui
MUSICWASP_SCREENSHOT=/tmp/undo.png MUSICWASP_UNDO=1 cargo run -p musicwasp-gui
```

`MUSICWASP_DATA_DIR=/some/dir` isolates a test library from your real one.

On Wayland, MusicWasp now uses the native backend and accepts file drops through winit's
`wl_data_device` support. Text copy/paste uses winit's data device too, which matters on compositors
such as Hyprland that only send the selection to a client's first data device. If a compositor
misbehaves, `MUSICWASP_X11=1` forces the old X11/XWayland path for that launch.

## Roadmap

Planned improvements:

1. **AcoustID fingerprinting** on top of the existing auto-tag (needs a key + `fpcalc`).
2. **Editing existing smart playlists** (currently create/delete only) plus rating and play-count
   fields to build rules on.
3. **Device sync** (MTP / Android) and podcast + internet radio support.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
