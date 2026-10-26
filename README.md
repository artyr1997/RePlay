# ReSound

A quiet music player for macOS in the style of ReNote. Rust (Tauri) scans your music
folder, keeps the library and playlists in SQLite and plays the audio (rodio); the
interface is Svelte.

- **Albums · Artists · Songs · Lists**: two columns (group → songs), or a song list
- **Player**: PLAY / PAUSE, PREV / NEXT, SHUF, REPT (all / one), QUEUE (up next),
  click the line to seek. The next song starts from Rust, also with the window hidden.
- **Playlists**: create, rename, delete; add songs or whole albums; remove songs
- **Actions** (E or right click): play, play next, add to queue, add to playlist, show in Finder
- Files: MP3, AAC / M4A, ALAC, FLAC, WAV, AIFF, OGG. Without tags, an
  `Artist / Album / 01 Song.mp3` layout is read from the folder and file names.
- Keys: Space play/pause · ⌘← ⌘→ previous/next · ↑ ↓ ← → move · Enter play ·
  1–4 tabs · / search · S shuffle · R repeat · Q queue · N new playlist · I info

## Run

```sh
npm install
npm run desktop        # dev window with live reload
npm run desktop:build  # ReSound.app + .dmg in src-tauri/target/release/bundle/
```

Rust tests: `cd src-tauri && cargo test`. The music folder defaults to `~/Music`
(change it in INFO). Library data: `~/Library/Application Support/com.artursavytskyi.resound/`.
