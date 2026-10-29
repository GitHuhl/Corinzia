# Corinzia

## A Hybrid DAW/Music Notation Program with CLAPi and VST3 Instrument Support
[![Listed on Remade](https://remade.wiki/badges/listed-on-remade.svg)](https://remade.wiki/projects/corinzia/)

Note: Corinzia is under active development and is considered pre-alpha quality software by its developer at this point in time.

<img width="1280" height="860" alt="image-1792010138652" src="https://github.com/user-attachments/assets/957cb598-5eaf-4b5c-ae24-09278e3d42c7" />
<img width="1280" height="860" alt="image-1792010442842" src="https://github.com/user-attachments/assets/5c9861a4-19e2-4038-85a9-016a5d3afff5" />



Corinzia is written in Rust. The score is engraved by its own engraver
(Bravura/SMuFL) and drawn on the GPU with iced; playback hosts CLAP and
VST3 instruments in a lock-free realtime engine, rendering instruments in
parallel on every core, that outputs through PipeWire.

* **Score tab:** Page and Panorama views, vector-sharp at any zoom. Note
  input from the keyboard, the mouse and the Notes panel; passages by
  marquee, Shift-click, a click in a bar's empty space or Ctrl+A;
  copy/paste/repeat; articulations, ornaments, ties, hairpins and dynamics
  (Shift+D types any of pppp…ffff, sf, sfz, fp, rfz…); slurs that nest,
  overlap and cross between voices and staves, placed and curved as
  LilyPond does it; click one and drag an end to another note (or
  Shift/Alt+←→), X flips it; tuplets of any ratio and grace notes; key,
  time (with pickups), clef, tempo, barlines, repeat endings, rehearsal
  marks and text through popovers and the Structure panel; popovers open
  beside what they act on. Double-click a title, tempo mark, text, chord
  symbol, dynamic or lyric to retype it; right-click a clef, key or time
  signature or barline to change it (a right-drag pans).
  Escape or a click on blank paper deselects. Every action is one undo
  step.
* **Players** (toolbar): add instruments (quick-add buttons, or a list
  such as "Flute, 2 Violin" with any name), remove players with their
  music, and reorder them; Apply makes it one undo step. New players get
  staves of rests in every movement and a mixer track of their own. Retype
  a player's instrument to change it (the music stays); add doublings after
  a comma ("Flute, Piccolo"), each with its own mixer track.
* **Instrument changes:** Shift+I (or Structure → Instrument change) at a
  note or bar switches the player to a doubling from there on, adding it if
  needed. The name is written above the staff, the clef follows, playback
  moves to that instrument's track, and the transposed score changes key
  at the next barline.
* **Transpose** (Ctrl+Shift+T): by an interval ("up M2", "down P5",
  "-m3", "octave") or to a key ("to Eb", the nearer way), spelt by the
  interval. A selected passage moves alone; otherwise the whole score,
  with its key signatures and chord symbols.
* **Select only and Select similar:** Ctrl+F picks, in the selected
  passage or the whole movement, what a few words describe: "top",
  "bottom", "chords", "voice 2", "above C5", "below G3", "F#" or "Bb4",
  "staccato", "tied", "rests", "dynamics mf", "lyrics verse 2", "chord
  symbols", "text dolce". Ctrl+Shift+A picks what is like the selected
  element (notes on its staff, the same dynamic, verse or text). Delete,
  articulations, dynamics, accidentals, ↑/↓ and Transpose apply to each
  picked element, as one undo step.
* **Explode and reduce** (Structure panel): with a passage selected over
  several staves, Explode spreads one staff's chords over them, top note
  to the top staff (a staff short of a note doubles the lowest; extra notes
  stay on the bottom staff), and Reduce combines them into chords on an
  empty staff among them (else the top one), tying notes held across
  others' attacks.
* **Beams:** the time popover takes a grouping ("7/8 2+2+3", "3+2/8");
  5/8, 7/8 and 8/8 group as LilyPond does by default. The Notes panel's
  Beams buttons break a beam before the selected notes, join them to the
  beam before (across beat groups), leave them flagged, or go back to
  automatic.
* **Percussion:** Drum set, Snare drum, Bass drum and Cymbals use the
  percussion clef, a drum map (staff position → sound, MIDI key and
  notehead: x, circled x, diamond, triangle) and MIDI channel 10. The
  status bar names the drum under the selection; playback sounds the
  mapped key. Drum staves have no key signature or accidentals.
* **Cues and part-only markings:** Ctrl+Shift+V pastes the clipboard's
  first staff as a cue: small, silent notes in voice 2 under the source
  instrument's name. The Notes panel's ᶜ button makes notes cue-size (or
  back). A text typed as "parts: solo" or "score: tutti" shows only in the
  parts or only in the score.
* **Cross-staff notes:** Ctrl+Shift+↓/↑ (or ⤓/⤒ in the Notes panel) writes
  the selected notes on the player's staff below or above, in that staff's
  clef; a beam with notes on both staves runs between them.
* **Condensing** (Layout → Score → Condense): in the score, neighbouring
  players of one instrument ("Flute 1", "Flute 2") share a staff named
  "Flute 1.2", the first player stems up and the second down. A bar they play
  alike is written once under "a 2", and the second player's resting bars
  disappear. Parts are never condensed; clicks on a shared staff edit the
  first player.
* **Guitar tablature:** Electric guitar, "Guitar (tab)" and Bass guitar
  get a tab staff under their staff, with standard tuning (E A D G B E, or
  E A D G). Structure → Tablature turns it on or off for any staff. Each note
  goes on the highest string that can play it, and the string line breaks
  around the fret number.
* **MusicXML round trip:** a player's staves export as one part (`<staves>`,
  cross-staff notes by `<staff>`). Export and import also carry hand-set
  beams (`<beam>`, compared with Corinzia's own grouping), grouped meters
  ("2+2+3"), cue notes, system and page breaks, instrument changes (one
  `<score-instrument>` each), drum sets (unpitched notes, noteheads and
  their sounds), guitar tunings (a TAB staff from another program becomes
  the tuning), and the subtitle, lyricist, arranger and copyright.
* **Preferences** (⚙ in the toolbar, Ctrl+,): theme (dark, light, high
  contrast), audio output device, sample rate, buffer size, and extra plugin
  folders. They're saved in `~/.config/corinzia/preferences.json`
  (`%APPDATA%\Corinzia` on Windows). Changing the device, rate or buffer
  restarts the audio, and plugins keep their settings.
* **Safe plugin scanning:** each plugin bundle is opened by a child process
  (`corinzia --scan-plugin <bundle>`) with a 30-second limit, so a plugin
  that crashes or hangs is listed as skipped instead of taking Corinzia
  down. Results are cached in `plugins.json` beside the preferences and
  reused until a bundle changes. Rescan plugins starts afresh.
* **Effects, buses and the master** (Mix tab): each strip has an effect
  chain (CLAP or VST3 effects, or the built-in Corinzia Gain, Reverb, EQ
  (low and high cut, shelves, two bells) and Limiter (look-ahead, never
  over its ceiling)), and effects can be bypassed or reordered. Tracks can also go to
  a bus instead of the master, and send to buses post-fader. Buses have
  their own effects, fader, pan and mute; the master strip has effects and
  a fader. A selected effect shows its editor button and a slider per
  parameter. Sends can be pre-fader (a monitor mix); buses can go into
  other buses (sections into a group), and solo works through them (a
  soloed bus keeps what feeds it heard, a soloed track the buses it
  reaches). Tracks have a colour and a folder; the arrange view shows a
  folder's tracks together and folds them away with a click on its header.
* **Presets and programs:** any plugin's sound (an instrument's or an
  effect's) can be saved as a named preset and loaded into any project.
  Presets live in `presets/<plugin>/` beside the preferences. VST3 plugins
  now keep their state too. A track's MIDI program (General MIDI names,
  optional bank) is sent to its instrument when playback starts, and is
  written into exported MIDI files. Corinzia Sine has three programs: soft
  organ, pure sine and bright.
* **Cycle:** ⟲ in the toolbar (Alt+L) cycles the selected passage, or
  turns the cycle on and off; Shift+drag along the arrange view's ruler
  sets it. Playback goes round it, notes released at each turn.
* **Audio tracks** (Mix tab): "+ Audio track", then "Add sound…" (or drop
  a WAV, FLAC, MP3 or Ogg file on the window) puts a clip at the caret.
  Clips sit at fixed times (seconds after their movement's start: tempo
  changes do not move them). In the arrange view, drag a clip to move it
  (snapping to the beat; Shift for none), its edges to trim, its top
  corners to fade; Ctrl+drag copies, Delete removes. A selected clip has
  its gain, mute and name beside the strips, and can be cued in the
  score: its name with ▸ on a chosen player's staff where it starts
  (through the tempo), optionally with a line to where it ends. Audio tracks have effects,
  sends, an output and a fader like any track. Files are decoded and
  resampled to the engine's rate once and kept in memory.
* **Automation** (Mix tab): a strip's "Automation lane" shows a curve on
  the track's lane in the arrange view: its gain, its pan, or any parameter
  of its instrument or effects. Click to add a point, drag to move it,
  right-click to delete it, Shift+click to make it hold until the next
  point (a step); values otherwise run in straight lines between points.
  Besides gain, pan and plugin parameters, a lane can drive a send's
  level, any MIDI controller (CC0–127) or pitch bend; buses and the master
  have lanes too (under "Buses", from their strips' "Automation lane").
  Automation follows repeats and is picked up when playback starts
  mid-piece. A track's "Dynamics" can also send the performed dynamics and
  hairpins as CC1 or CC11 for sample libraries, during playback and in
  exported MIDI files (Corinzia Sine follows CC11 and CC7).
* **Playing techniques (expression maps):** a track's "Playing techniques"
  table (Mix tab, select the strip) maps techniques to a keyswitch, a program
  change, a controller value or a MIDI channel. The techniques are technique
  texts (pizz., arco, con sord., sul pont., trem., …; the last one lasts) and
  articulations (staccato, accent, legato, …; for that note). "ord." is the
  normal sound. Switches are sent just before the notes that need them and
  only when the sound changes, during playback and in exported MIDI files.
* **MIDI keyboard input** (Preferences → MIDI keyboard): in note input,
  each key played is entered at the caret with the current duration,
  spelled in the key signature. Keys held together make a chord, and on a
  drum set a key picks the drum mapped to it. Keys also sound on the
  caret's instrument. On Linux the raw MIDI ports (`/dev/snd/midiC*D*`)
  are read directly, with no library needed; Windows uses WinMM.
* **Layout** (Structure → Page → Layout…): page size (A4, Letter, A3, B4,
  Legal, Tabloid, A5 or custom) and orientation, staff size, margins, note
  spacing, staff and system distances, bars per system and page numbers,
  for the full score and, separately, for the parts; and the page texts:
  subtitle, lyricist, arranger and copyright. The view and PDF export
  follow them. Enter puts a system break before the selected bar,
  Ctrl+Enter a page break (again removes it); shown a part, the break is
  that part's alone. Later systems name staves by their short names, set
  in Players as "Violin I / Vln. I".
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
* **Mix tab** (Ctrl+2): the arrange view, mixer strips (faders with a dB
  scale, pan, mute, solo; double-click resets, Shift drags finely) and a
  browser of the CLAP and VST3 instruments found in `CLAP_PATH`, `~/.clap`,
  `/usr/lib/clap`, `VST3_PATH`, `~/.vst3`, `/usr/lib/vst3` and your Nix
  profiles. Click a strip's instrument to open the plugin's own editor in a
  window and change its sound; click again (or close the window) to close
  it. Click the track name to select the strip without opening anything.
  Plugin settings are not saved with the project yet.
* **Files:** New (a template or an instrument list, e.g. "Flute, 2 Violin,
  Cello", with title, key, time and pickup, tempo and length),
  Open, Save and autosave to a single SQLite `.project` file. Import (or
  Open) reads MusicXML (`.musicxml`, `.xml`, compressed `.mxl`). Where the
  desktop has no file-dialog portal, Corinzia's own file browser opens
  instead (`CORINZIA_FILE_DIALOG=builtin` always uses it). Export writes
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
(Vulkan), PipeWire for audio, and the CLAP and VST3 plugins installed in
`~/.clap`, `/usr/lib/clap`, `~/.vst3` or `/usr/lib/vst3`. Plugin editors
open as X11 windows (through XWayland on Wayland desktops). Without
PipeWire it still runs, without sound. On NixOS
use the flake below, or `appimage-run`.

**Windows (x86-64):** get `Corinzia-…-x86_64-windows.zip`, unzip it and run
`corinzia.exe`, keeping the `fonts` folder next to it. Audio goes to the
default output device (WASAPI). CLAP instruments are found in
`%LOCALAPPDATA%\Programs\Common\CLAP` and `%COMMONPROGRAMFILES%\CLAP`, VST3
instruments in `%LOCALAPPDATA%\Programs\Common\VST3` and
`%COMMONPROGRAMFILES%\VST3`.

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
nix build .#release                           # both: result/ holds the AppImage and the zip

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
| `rust/corinzia-audio` | CLAP and VST3 host, plugin editors, realtime engine, PipeWire output |
| `rust/corinzia-sine` | Corinzia Sine, the built-in CLAP instrument |
| `rust/corinzia-export` | MIDI and MusicXML writers |
| `rust/corinzia-score` | exact fractions, note values, the engraver's input types |
| `assets/fonts/bravura` | Bravura (SIL OFL) and its SMuFL metadata |
| `scripts/` | headless screenshots, a private PipeWire session for the live audio test |

Press **N** to write notes: A–G enter pitches, 1–7 pick durations
(5 = quarter), R enters a rest; clicking a staff in input mode places a note,
and a shadow of the note under the pointer shows where and at which pitch.
Outside note input the same keys edit the selection: 1–7 and `.` change
the selected notes' durations (longer music overwrites what follows;
shorter leaves rests), A–G change the pitch, `-` `=` `0` set flat, sharp or
natural, Alt+←/→ move notes by the rhythmic grid chosen in the status
bar (Alt+[ / Alt+] change it). Shift+M "pickup 1/4" adds an upbeat bar.
Shift+C also takes "8va", "8vb", "15ma", "loco"; Shift+P "ped" (and "*")
puts a pedal under the selection. Shift+R also takes jumps: "segno", "coda", "to coda", "fine", "D.C. al
Fine", "D.S. al Coda" (played back too). Shift+F types fingering ("1", or "1 3 5" for a chord); "up bow" and "down
bow" go in Shift+O or Shift+P. `;` makes tuplets ("3", "5", "5:4", "7:8"): in note input the notes you
type go into them until `:`; with a note selected it starts one. `/` in
note input types grace notes (slashed; `?` plain) before the next note.
Popovers: Shift+K key ("Eb", "f#" minor), Shift+M time ("6/8"), Shift+T
tempo ("Allegro 132"), Shift+B barline (":|"), Shift+C clef, Shift+X /
Shift+P expression or technique text, Shift+O ornaments, Shift+R endings,
Shift+A rehearsal mark; Ctrl+B inserts a bar. The full key map is in
`rust/corinzia-app/src/keys.rs`.

**Parts:** the toolbar's picker shows one player's part, as written, with
runs of empty bars as multi-bar rests; it edits like the score. Export →
Parts (PDF) prints every part. `--part N` opens on player N's part.

**Movements:** "All movements in view" (Structure panel) shows them one
after another on the pages, as printed; a click on one edits it. The
Structure panel lists them (new, rename, delete);
Ctrl+PageUp/PageDown or the arrows in the status bar switch. PDF export
prints the whole work, each movement from a new page.

Not yet: bus routing and plugin delay compensation (tracks mix straight to
the output), automation playback and saving plugin settings.
