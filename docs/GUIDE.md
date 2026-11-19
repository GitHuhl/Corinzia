# Corinzia user guide

Everything Corinzia does, area by area. Press **F1** in the app for the
keyboard shortcuts.

## Writing notes

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

## Features

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
* **Byzantine chant** (Structure → Byzantine chants): chants in
  Chrysanthine notation, shown in the Score tab in place of the movement
  and engraved with the Neanes font: the mode signature, neumes with their
  time signs, gorgons and fthoras, martyriae, and Greek syllables with
  hyphens and melismas, on justified lines. Type neumes at the caret (0–5
  up, A E C down, Y P K…), `.` G D for time, F a fthora on the neume's note,
  M a martyria, Ctrl+L syllables as a stream, Shift+M the mode ("πλ. β΄");
  the Notes panel becomes a neume palette, and F1 lists the keys. The melody
  is worked out in the 72 moria of each genus, and a martyria that does not
  match it is named in the status bar. Export → PDF prints the chant shown;
  `--chant N` opens on chant N. Space sings the chant from the caret on
  the first player's instrument, every note at its pitch in moria (the
  nearest key, pitch-bent; the built-in instruments follow the usual ±2
  semitone bend), with the ison held an octave under the mode's base note (Shift+D
  turns it off); the neume being sung is highlighted. Chants are saved
  with the project.
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
  written into exported MIDI files. The built-in instruments take General
  MIDI programs (bank 1: string sections and choir; bank 128: drums), so a
  track's program can choose another of their sounds.
* **Cycle:** ⟲ in the toolbar (Alt+L) cycles the selected passage, or
  turns the cycle on and off; Shift+drag along the arrange view's ruler
  sets it. Playback goes round it, notes released at each turn.
* **Metronome and count-in:** Click in the toolbar (Alt+M) clicks every
  beat while playing (a dotted quarter in 6/8, 9/8, 12/8; the downbeat
  higher). Preferences → Count-in plays one or two bars of clicks, at the
  tempo where playback starts, before it does. Neither is in exports.
* **Where you are:** the toolbar shows the bar and beat, the time into
  the movement and the tempo: the playhead's while playing, the caret's
  otherwise. Ctrl+G (or `goto 25`, `goto B`) goes to a bar or a rehearsal
  mark, in the score and the arrange view.
* **Recent scores:** the Recent menu beside Open lists the last eight
  scores opened or saved.
* **The Mix tab's panes:** the wheel scrolls the arrange view's tracks,
  Shift+wheel along time, Ctrl+wheel zooms; Alt+wheel or dragging a
  track's bottom edge (in the headers) makes the tracks taller or
  shorter; drag the headers' edge to widen them. Each header has mute (M)
  and solo (S) buttons. Drag the bar between the arrange view and the
  mixer, or before the side panel, to resize them; the faders grow with
  the mixer. The sizes are remembered. A strip's automation lane and
  dynamics controller are in the side panel when the strip is selected.
* **Audio tracks** (Mix tab): "+ Audio track", then "Add sound…" puts a
  clip at the caret. Or drag WAV, FLAC, MP3 or Ogg files in from your
  file manager (Wayland and X11 alike): dropped on an audio track in the
  arrange view, the first goes on that track at the beat under the
  pointer and any others each on a new track there; dropped anywhere
  else, each goes on a new track at the caret. While you drag, the
  arrange view highlights the lane and beat it will land on ("New audio
  track" over empty space), and anywhere else the window is outlined. Clips sit at fixed times (seconds after their movement's start: tempo
  changes do not move them). In the arrange view, drag a clip to move it
  (snapping to the beat; Shift for none), its edges to trim, its top
  corners to fade; Ctrl+drag copies, Delete removes. S (or "Split" by
  the strips) splits the selected clip at the playhead, or at the caret
  when stopped (click the ruler to put it there); Alt+click on a clip
  splits it right there, on the beat (Shift too: anywhere). The two
  halves play as the one did: the fade in stays on the first, the fade
  out on the second.
* **Tempo from a recording** (Mix tab, a clip selected): "Follow this
  clip" finds the clip's beats and sets the movement's tempo so that the
  score's beats fall on them: the bars, the built-in orchestra and the
  clips' places in the score then line up with the recording. Set the
  score's tempo roughly first (it tells Corinzia how fast the beat is,
  so it counts beats, not half beats); the beat is the meter's (a dotted
  crotchet in 6/8), or choose one. "Every beat" puts each beat where it
  was played; "Each bar" puts the downbeats there and spaces the beats
  between them evenly. The tempo changes this makes play but are not
  engraved; tempo marks in the stretch stay, with the tempo they now
  have. The phrasing rules' slowing and fermata holds leave that stretch
  alone (the recording has its own). One undo step; "Forget" returns to
  the tempo marks. Not yet in movements with repeats. A selected clip has
  its gain, mute and name beside the strips, and can be cued in the
  score: its name with ▸ on a chosen player's staff where it starts
  (through the tempo), optionally with a line to where it ends. Audio tracks have effects,
  sends, an output and a fader like any track. Files are decoded and
  resampled to the engine's rate once and kept in memory.
* **Audio tracks in the score:** each audio track with clips in the
  movement has a staff of its own under the players', named after the
  track: one line, with no clef, key or time signature and no rests.
  Each clip shows its file's name in bold in a box where it starts, with
  a heavy line along the staff to where it ends. A muted clip is left
  out. Players' parts don't show these staves (use a cue for that).
* **Automation** (Mix tab): a strip's "Automation lane" (in the side panel
  when the strip is selected) shows a curve on
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
  exported MIDI files (the built-in instruments follow CC1, CC7 and CC11).
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
  performed. Tracks without a plugin play through **Corinzia Ensemble**,
  the built-in instruments (also installed as
  `lib/clap/Corinzia Ensemble.clap` for other hosts). Entered notes are
  auditioned. If the computer cannot render a block in time (a click or a
  gap), the status bar says so, with the peak load; a larger buffer in the
  preferences gives it more time. `get status` reports the count too.
* **The demo:** `corinzia` without a file opens *A Festival Prelude*, an
  original piece for full orchestra (made by `scripts/festival-prelude.py`)
  that shows the built-in instruments: long string lines, woodwind solos
  over pizzicato, fast staccato strings with brass fanfares, and a brass
  chorale. Its master has a little gain off, a hall and a limiter.
* **Built-in instruments:** every staff plays a sound chosen by its
  instrument's name: strings (sections in an orchestral score, soloists in
  chamber music), woodwinds, brass, voices and choir, piano, harpsichord,
  organ, harp, guitar, tuned percussion, timpani and drums: 47 sounds.
  Strings, woodwinds and brass combine each instrument's spectrum (how its
  harmonics sit at each dynamic and register), a physical model of the
  bow, reed or lips and the tube or string they drive, and filtered noise
  for bow and breath. They are balanced as an orchestra is: with every
  fader at 0 dB a trumpet's ff is far louder than a flute's, a solo violin
  sits under a section, and there is headroom for a full tutti. They play
  what the score asks: hairpins through held notes, slurred notes joined
  (legato, no new attack), accents, and the techniques pizz., trem., con
  sord., sul pont., sul tasto, col legno, harmonics and flutter-tonguing.
  Fermatas are held. Articulations change how each note speaks: staccato
  lasts about a fifth of a second and staccatissimo a tenth, whatever the
  tempo; quick unslurred notes (under a fifth of a second) are played off
  the string or lightly tongued; accents and marcato start brighter and
  louder and settle to the dynamic, as an *sfz* or *fp* does; tenuto
  starts gently; unslurred string notes alternate down-bows and up-bows
  (or follow the bowing marks); a wide slurred leap on a string slides;
  a tremolo is one bowed note, not repeated notes.
* **Phrasing:** the toolbar's Phrasing button (or `phrasing on|off`) plays
  the score as players shape it, after the KTH rules for music
  performance: each phrase swells and relaxes, the last bar slows like a
  runner stopping, the beat before a fermata broadens, downbeats and
  melodic peaks are leant on, higher notes of a line are a little louder,
  short notes lighter, a wide unslurred leap gets a small break, and the
  top of a chord sings over the rest. Off, the score plays as written. New
  scores start with it on; the choice is saved with the project, and
  applies to exported MIDI and audio too.
* **Shared instruments:** a track can play through another track's plugin
  instance (Mix tab, the track's panel: "Share an instrument"), each on a
  MIDI channel of its own: one multi-timbral sampler for a section rather
  than one copy per track. The first such track's strip carries the
  instance's sound; the others' faders and pans become MIDI volume (CC7)
  and pan (CC10) on their channels.
* **Piano roll** (Mix tab: double-click a track's lane): the track's notes
  as they play, over the movement. Drag a note to play it earlier or later
  (Shift: by sixteenths), its right edge to hold it longer or shorter, and
  the velocity lane underneath for how hard it is played; a tweaked note
  is outlined, R (or "Reset") plays the selection as written again. These
  are kept apart from the notation: pitch and rhythm stay the score's, and
  the changes apply on every pass through repeats.
* **Meters:** every strip (tracks, buses, the master) has a stereo peak
  meter beside its fader, on the fader's scale; a light on top latches
  when it goes over 0 dB (click it to put it out).
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
  a vector PDF straight from the engraver, MIDI and MusicXML 4.0, and
  audio: the movement as it plays, mixed to a 24-bit WAV (rendered offline
  by its own plugin instances, with the reverb tail; Escape stops it;
  `--export out.wav` works headless too).
  `corinzia score.project --export score.pdf` works headless.

## Scripting and AI agents

Every edit can be made from the command line, without opening a window.
Commands run in order, each answering with one line of JSON
(`{"ok":true,"result":…}` or `{"ok":false,"error":…}`); the program exits
with status 1 if one fails.

```bash
corinzia score.project --do "select bar 3-4 staff 1" --do "transpose \"up M2\"" --do save
```

```bash
corinzia score.project --do "get notes bar 1-2 staff 1"
```

* `--do "<command>"` runs one command (repeat it for more);
  `--script file.txt` runs a file of them, one per line (`#` starts a
  comment); `--stdin` reads commands from standard input until it ends.
  `--export out.pdf` afterwards exports the result.
* `get project`, `get notes [bar A-B] [staff S]`, `get selection`,
  `get chant` and `get status` answer with JSON.
* `act <action>` runs any keyboard or menu action as JSON (`act "Undo"`,
  `act {"Duration":2}`), and `popover <kind> <text>` types into any
  popover (`popover Tempo "Allegro 132"`), so everything the interface
  can do, a script can do. `help` lists the readable commands (`note`,
  `key`, `select bar`, `save`, `neume`…).

**The mixer** has commands of its own, naming strips by their track or bus
name (or `master`, or a track's number), plugins by name and effects by
their place in the chain:

```bash
corinzia score.project --do "bus add Strings" --do "output cello strings" --do "send flute strings -12 pre" --do "effect add strings Corinzia Reverb" --do "param strings 1 Mix 0.4" --do "automate flute gain at 5:3 -6" --do save
```

`gain`, `pan`, `mute`, `solo`, `plugin`, `effect add|remove|bypass|move`,
`param`, `bus add|remove`, `output`, `send`, `program`, `color`, `folder`,
`automate` (gain, pan, bend, cc, send or an effect parameter, at a bar and
beat) and `audio track|add`; `get mixer`, `get plugins` and
`get params <strip> <effect>` answer with JSON, and `mix <message>` runs
any Mix tab message as JSON.

**MCP:** `corinzia --mcp score.project` is a Model Context Protocol
server on standard input and output, for AI assistants. Its tool,
`corinzia`, runs one command and answers with its JSON. With Claude Code:

```bash
claude mcp add corinzia -- corinzia --mcp /path/to/score.project
```

## Project files

The `.project` format is still changing. Each file records its format
version, and a version of Corinzia opens only its own: a file from an
earlier or later version is refused (never misread or overwritten), so keep
MusicXML exports of anything you need to carry across versions.
