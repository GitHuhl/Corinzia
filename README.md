
# Corinzia

## A Hybrid DAW/Music Notation Program with CLAPi and VST3 Instrument Support

<img width="1280" height="860" alt="image-1792010138652" src="https://github.com/user-attachments/assets/957cb598-5eaf-4b5c-ae24-09278e3d42c7" />
<img width="1280" height="860" alt="image-1792010442842" src="https://github.com/user-attachments/assets/5c9861a4-19e2-4038-85a9-016a5d3afff5" />



Corinzia is written in Rust. The score is engraved by its own engraver
(Bravura/SMuFL) and drawn on the GPU with iced; playback hosts CLAP
instruments in a lock-free realtime engine that outputs through PipeWire.

* **Score tab:** Page and Panorama views, vector-sharp at any zoom. Note
  input from the keyboard, the mouse and the Notes panel; passages by
  marquee, Shift-click or Ctrl+A; copy/paste/repeat; articulations,
  ornaments, dynamics, slurs, hairpins and ties; key, time, clef, tempo,
  barlines, repeat endings, rehearsal marks and text through popovers and
  the Structure panel. Every action is one undo step.
* **Lyrics and chord symbols:** Ctrl+L types lyrics syllable by syllable
  (Space moves to the next note, `-` hyphenates, `_` draws an extender;
  Ctrl+Shift+L for verse 2). Ctrl+K types chord symbols ("F#m7b5/E" is
  engraved as F♯m7♭5/E). Staves move apart where they need the room.
* **Playback:** Space plays from the selection or caret with a playhead in
  the score; repeats, endings, ties, ornaments, dynamics and hairpins are
  performed. Tracks without a plugin play through **Corinzia Sine**, a
  small built-in CLAP instrument (also installed as
  `lib/clap/Corinzia Sine.clap` for other hosts). Entered notes are
  auditioned.
* **Mix tab** (Ctrl+2): the arrange view, mixer strips (gain, pan, mute,
  solo) and a browser of the CLAP instruments found in `CLAP_PATH`,
  `~/.clap`, your Nix profiles and `/usr/lib/clap`.
* **Files:** New (type an instrument list, e.g. "Flute, 2 Violin, Cello"),
  Open, Save and autosave to a single SQLite `.project` file. Open also
  imports MusicXML (`.musicxml`, `.xml`, compressed `.mxl`). Export writes
  a vector PDF straight from the engraver, MIDI and MusicXML 4.0.
  `corinzia score.project --export score.pdf` works headless.

**Read the design:** [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)

## Download

**Linux:** get `Corinzia-…-x86_64.AppImage` from the
[Releases](https://github.com/githuhl/corinzia/releases) page, then:

```sh
chmod +x Corinzia-*-x86_64.AppImage
./Corinzia-*-x86_64.AppImage
```

It runs on x86-64 distributions with glibc 2.31 or newer (Ubuntu 20.04,
Debian 11, Fedora 32 and later). It uses your system's graphics driver
(Vulkan), PipeWire for audio, and the CLAP plugins installed in `~/.clap`
or `/usr/lib/clap`. Without PipeWire it still runs, without sound. On NixOS
use the flake below, or `appimage-run`.

**Windows (x86-64):** get `Corinzia-…-x86_64-windows.zip`, unzip it and run
`corinzia.exe`, keeping the `fonts` folder next to it. Audio goes to the
default output device (WASAPI). CLAP instruments are found in
`%LOCALAPPDATA%\Programs\Common\CLAP` and `%COMMONPROGRAMFILES%\CLAP`.

**This is a preview:** saving projects is turned off while the file format
is still changing. Open and import scores, edit, play, and export to PDF,
MIDI or MusicXML.

Found a bug? Please [open an issue](https://github.com/githuhl/corinzia/issues).

## Run, build & test (Nix only)

```sh
nix run                                       # the app (a demo score)
nix run . -- score.project                    # open a project
nix build                                     # sandboxed build running every test
nix build .#appimage                          # the portable AppImage (x86-64)
nix build .#windows                           # the Windows zip (cross-compiled)

nix develop                                   # pinned nixos-26.05 toolchain
cd rust && cargo test --workspace             # all crates
cargo run -p corinzia-app                     # the app from source
scripts/screenshot.sh out.png --keys "n c d e" # headless screenshot (Xvfb + lavapipe)
```

## Layout

| Path | What |
|---|---|
| `rust/corinzia-app` | the application (iced): score canvas, editor, panels, playback, Mix tab, files |
| `rust/corinzia-edit` | pure editing operations: note input, ranges, structure, popover parsing |
| `rust/corinzia-model` | project model and `.project` storage (SQLite, `schema/project.sql`) |
| `rust/corinzia-layout` | the engraver: display lists, page layout, PDF |
| `rust/corinzia-perform` | performance: repeats, ties, ornaments, dynamics; tempo maps; export documents |
| `rust/corinzia-audio` | CLAP host, realtime engine, PipeWire output |
| `rust/corinzia-sine` | Corinzia Sine, the built-in CLAP instrument |
| `rust/corinzia-export` | MIDI and MusicXML writers |
| `rust/corinzia-score` | exact fractions, note values, the engraver's input types |
| `assets/fonts/bravura` | Bravura (SIL OFL) and its SMuFL metadata |
| `scripts/` | headless screenshots, a private PipeWire session for the live audio test |

Press **N** to write notes: A–G enter pitches, 1–7 pick durations
(5 = quarter), R enters a rest; clicking a staff in input mode places a note.
Outside note input the same keys edit the selection: 1–7 and `.` change
the selected notes' durations (longer music overwrites what follows;
shorter leaves rests), A–G change the pitch, `-` `=` `0` set flat, sharp or
natural, Alt+←/→ move notes by the rhythmic grid chosen in the status
bar (Alt+[ / Alt+] change it). Shift+M "pickup 1/4" adds an upbeat bar.
`;` makes tuplets ("3", "5", "5:4", "7:8"): in note input the notes you
type go into them until `:`; with a note selected it starts one.
Popovers: Shift+K key ("Eb", "f#" minor), Shift+M time ("6/8"), Shift+T
tempo ("Allegro 132"), Shift+B barline (":|"), Shift+C clef, Shift+X /
Shift+P expression or technique text, Shift+O ornaments, Shift+R endings,
Shift+A rehearsal mark; Ctrl+B inserts a bar. The full key map is in
`rust/corinzia-app/src/keys.rs`.

Not yet: bus routing and plugin delay compensation (tracks mix straight to
the output), automation playback, plugin GUIs, and more than one movement
in the view.
