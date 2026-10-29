# DAW feature parity (DAWproject as the reference)

What a DAW session holds, element by element, from the
[DAWproject 1.0](https://github.com/bitwig/dawproject) schema
(`Project.xsd`, `MetaData.xsd`), against what Corinzia has. DAWproject is
used only as a checklist of what DAWs that support it (Bitwig Studio,
Studio One, Cubase, VST Live, n-Track) agree a session is. Corinzia
neither reads nor writes the format.

Status: **yes** — has it; **partly** — some of it; **no** — missing;
**n/a** — not wanted, with the reason.

## Project and metadata

| DAWproject | Corinzia | Status | Notes |
|---|---|---|---|
| `MetaData`: Title, Composer, Arranger, Songwriter, Copyright, Comment… | `Project.title`, `composer`, `arranger`, `lyricist`, `subtitle`, `copyright` | partly | No Artist/Album/Genre/Year/Website/Comment; only Comment seems worth adding |
| `Application` name/version | — | n/a | |

## Transport and the timeline

| DAWproject | Corinzia | Status | Notes |
|---|---|---|---|
| `Transport/Tempo`, `Arrangement/TempoAutomation` (hold or linear points) | `Movement.tempo` (marks, with `ramp`) | yes | Tempo follows the score; ramps are linear |
| `TimeSignature`, `TimeSignatureAutomation` | `Movement.meter` | yes | |
| `Markers` / `Marker` (time, name, color) | rehearsal marks, navigation marks | partly | Only marks that are engraved; no DAW-only cue markers |
| `timeUnit` beats / seconds | written and performed quarters, samples; seconds for clips | yes | Audio clips sit at fixed times (seconds after their movement's start) |
| Loop range (`loopStart`/`loopEnd` on clips) | transport cycle | partly | A cycle (Alt+L, the ruler); no looped clips |
| `Scenes`, `ClipSlot` (clip launching) | — | n/a | A session/launcher view does not fit a notation program |

## Tracks and channels (`Structure`)

| DAWproject | Corinzia | Status | Notes |
|---|---|---|---|
| `Track` contentType `notes` | `Track` (an instrument played by the score) | yes | Notes come from the score, not from clips |
| `Track` contentType `audio` | audio tracks | yes | See "Audio tracks" below |
| `Track` contentType `automation` | `AutoLane` on a track | yes | |
| Nested `Track` (folders, groups) | `Track.folder` | partly | One level of folders, folding in the arrange view; group buses are buses into buses |
| `Track` / `Channel` `color`, `comment` | `Track.color` | partly | Colours; no comments |
| `Channel` `role` `regular` | track strip | yes | |
| `role` `effect` (return) | `Bus` fed by sends | yes | |
| `role` `submix` (group bus) | `Bus` as a track's or a bus's output | yes | Buses into buses, loops refused |
| `role` `vca` | — | no | Useful for sections; low priority |
| `role` `master` | `Master` | yes | Master has no pan (DAWs usually don't either) |
| `Volume`, `Pan`, `Mute` | `gain_db`, `pan`, `mute` (tracks; buses) | yes | |
| `solo` | `Track.solo`, `Bus.solo` | yes | Solo follows routing and sends |
| `destination` (any channel) | `output_bus` or the master | partly | Only one level of buses |
| `audioChannels` | stereo everywhere | partly | No mono or surround; mono audio files will need it (play mono as centred stereo at first) |
| `Send` volume | `SendLevel.gain_db` | yes | |
| `Send` `type` pre / post | `SendLevel.pre` | yes | |
| `Send` `Pan`, `Enable` | — | no | Small |

## Devices

| DAWproject | Corinzia | Status | Notes |
|---|---|---|---|
| `ClapPlugin`, `Vst3Plugin` | CLAP and VST3 hosting | yes | |
| `Vst2Plugin` | — | n/a | VST2 is no longer licensed |
| `AuPlugin` | — | n/a | macOS only; not a target |
| `deviceRole` `instrument` | track instrument | yes | |
| `deviceRole` `audioFX` | inserts on tracks, buses, master | yes | |
| `deviceRole` `noteFX` (MIDI effects) | expression maps (techniques) | partly | No note-effect plugins; the expression map covers the main use |
| `deviceRole` `analyzer` | — | no | A level meter per strip would be the useful part |
| `Enabled` | `AudioNode.bypassed` | yes | |
| `State` (preset file) | `Project.plugin_state`, presets | yes | |
| `Parameters` (real, bool, integer, enum) | plugin parameters, automatable | yes | |
| Built-in `Equalizer`, `Compressor`, `NoiseGate`, `Limiter` | Corinzia Gain, Reverb, EQ, Limiter | partly | No compressor or gate |

## Automation and expression (`Points`, `automationTarget`)

| DAWproject | Corinzia | Status | Notes |
|---|---|---|---|
| Target `parameter` (any device parameter) | `AutoTarget::Param` | yes | |
| Volume and pan automation | `AutoTarget::Gain`, `Pan`, `Send` | partly | Tracks, buses, the master and sends; no mute automation |
| `interpolation` hold / linear | `AutoPoint.hold` | yes | |
| `channelController` (CC) | `AutoTarget::Cc`; dynamics as CC1/CC11; expression maps | yes | Any controller, on the track's channel |
| `programChange` | track program and bank; expression maps | yes | |
| `pitchBend` | `AutoTarget::PitchBend` | yes | |
| `channelPressure`, `polyPressure` | — | no | |
| Per-note expression (`Note` with nested `Points`: gain, pan, transpose, timbre, pressure) | — | no | CLAP note expressions; could follow from articulations later |
| `Note` `vel`, `rel` (release velocity) | velocity from dynamics | partly | No release velocity |

## Clips and media (`Clips`, `Clip`, `Audio`, `Warps`)

| DAWproject | Corinzia | Status | Notes |
|---|---|---|---|
| `Clip` `time`, `duration` | `AudioClip.at`, `length` | yes | Seconds after the movement's start |
| `playStart` / `playStop` (trim) | `offset`, `length` | yes | Trimmed by dragging an edge |
| `fadeInTime` / `fadeOutTime` | `fade_in`, `fade_out` | yes | Linear; dragged from the top corners |
| `enable`, `name`, `color`, `comment` | `mute`, `name` | partly | Clips take their track's colour |
| `loopStart` / `loopEnd` (looped clips) | — | no | |
| `Audio`: file, `channels`, `sampleRate` | `file`; decoded mono or stereo, resampled | yes | WAV, FLAC, MP3, Ogg Vorbis; held in memory, not streamed from disk |
| `Warps` / `Warp`, `algorithm` | — | no | Clips play at their own speed |
| `Video` | — | no | |
| Clips on note tracks (MIDI clips) | — | n/a | Notes come from the score |

## Audio tracks: decisions so far

* **On the page:** audio is not engraved by default (option 1). A clip
  can optionally show a cue mark on a chosen staff: its name at its
  start ("Tape 3 ▸"), optionally with a line to its end (option 2). A
  dedicated tape/electronics staff is not planned.
* **In the arrange view (Mix tab):** audio tracks are lanes with clips
  drawn as blocks with their waveform. Clips are dragged along the
  timeline to move them (snapping to the beat grid, Shift for no snap),
  dragged at either edge to trim, and dragged at a top corner to fade.
  Click selects; Delete removes; Ctrl+drag duplicates.
* **In the mixer:** an audio track is a strip like an instrument track
  (inserts, sends, output bus, fader, pan, mute, solo, automation).
* **Anchoring:** fixed (clock) time: seconds after the movement's start
  as performed. Tempo changes and repeats do not move a clip; its place
  in the arrange view and its cue in the score follow the tempo.

## Next

Done so far: audio tracks and cues, the mixer and automation gaps, the
EQ and limiter, the cycle. Still open from the map: streaming long files
from disk, warping, looped clips, video, per-note expression, release
velocity, VCAs, a compressor and gate, analyzers (level meters), comments.
