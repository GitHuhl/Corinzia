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
| `timeUnit` beats / seconds | written and performed quarters, samples | partly | Everything is in musical time; nothing anchored to clock time (needed for audio, below) |
| Loop range (`loopStart`/`loopEnd` on clips) | — | no | No transport loop either; a cycle region is standard fare |
| `Scenes`, `ClipSlot` (clip launching) | — | n/a | A session/launcher view does not fit a notation program |

## Tracks and channels (`Structure`)

| DAWproject | Corinzia | Status | Notes |
|---|---|---|---|
| `Track` contentType `notes` | `Track` (an instrument played by the score) | yes | Notes come from the score, not from clips |
| `Track` contentType `audio` | — | **no** | The main gap: see "Audio tracks" below |
| `Track` contentType `automation` | `AutoLane` on a track | yes | |
| Nested `Track` (folders, groups) | — | no | Worth having for orchestral templates (strings/winds/brass folders) |
| `Track` / `Channel` `color`, `comment` | — | no | Colors help a lot in the arrange view at 60+ tracks |
| `Channel` `role` `regular` | track strip | yes | |
| `role` `effect` (return) | `Bus` fed by sends | yes | |
| `role` `submix` (group bus) | `Bus` as a track's output | yes | A bus cannot output to another bus yet |
| `role` `vca` | — | no | Useful for sections; low priority |
| `role` `master` | `Master` | yes | Master has no pan (DAWs usually don't either) |
| `Volume`, `Pan`, `Mute` | `gain_db`, `pan`, `mute` (tracks; buses) | yes | |
| `solo` | `Track.solo` | partly | Buses have no solo |
| `destination` (any channel) | `output_bus` or the master | partly | Only one level of buses |
| `audioChannels` | stereo everywhere | partly | No mono or surround; mono audio files will need it (play mono as centred stereo at first) |
| `Send` volume | `SendLevel.gain_db` | yes | |
| `Send` `type` pre / post | post-fader only | partly | Pre-fader sends are needed for monitor/headphone mixes; easy to add |
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
| Built-in `Equalizer`, `Compressor`, `NoiseGate`, `Limiter` | Corinzia Gain, Corinzia Reverb | no | An EQ and a limiter on the master are the ones users expect from a DAW |

## Automation and expression (`Points`, `automationTarget`)

| DAWproject | Corinzia | Status | Notes |
|---|---|---|---|
| Target `parameter` (any device parameter) | `AutoTarget::Param` | yes | |
| Volume and pan automation | `AutoTarget::Gain`, `Pan` (tracks) | partly | Not on buses, the master or sends; no mute automation |
| `interpolation` hold / linear | linear only | partly | Hold (steps) is needed for switches and enum parameters |
| `channelController` (CC) | dynamics as CC1/CC11; expression-map CCs | partly | No free CC lanes (CC21 vibrato, CC2…) |
| `programChange` | track program and bank; expression maps | yes | |
| `pitchBend` | — | no | |
| `channelPressure`, `polyPressure` | — | no | |
| Per-note expression (`Note` with nested `Points`: gain, pan, transpose, timbre, pressure) | — | no | CLAP note expressions; could follow from articulations later |
| `Note` `vel`, `rel` (release velocity) | velocity from dynamics | partly | No release velocity |

## Clips and media (`Clips`, `Clip`, `Audio`, `Warps`)

These describe audio tracks, which Corinzia does not have yet.

| DAWproject | Status | For Corinzia's audio tracks |
|---|---|---|
| `Clip` `time`, `duration` | no | Required: position and length on the timeline |
| `playStart` / `playStop` (trim) | no | Required: trimming either edge |
| `fadeInTime` / `fadeOutTime`, `fadeTimeUnit` | no | Required: fades, to avoid clicks |
| `enable` (mute a clip), `name`, `color`, `comment` | no | Name and mute required; the name is what the score shows |
| `loopStart` / `loopEnd` (looped clips) | no | Later |
| `Audio`: file reference (embedded or external), `channels`, `sampleRate` | no | Required: external files, any sample rate, resampled on load |
| `Warps` / `Warp` (time-stretch to the tempo), `algorithm` | no | Later; first version plays audio at its own speed |
| `Video` | no | Later (film scoring); same timeline model as audio |
| Clips on note tracks (MIDI clips) | n/a | Notes come from the score |

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
* **Open: what a clip's position is anchored to.** Musical time (bar and
  beat: follows inserted bars and tempo changes) or clock time (fixed
  seconds: for picture and fixed media). Proposed: musical time by
  default, clock time per clip as an option. Repeats: a musically
  anchored clip plays on each pass of a repeat.

## Suggested order

1. Audio tracks: model, file loading and playback in the engine, arrange
   view with drag/trim/fade, mixer strip.
2. Cue marks for clips on the page.
3. Mixer gaps: pre-fader sends, bus solo, buses into buses, folder tracks,
   track colors.
4. Automation gaps: hold interpolation, bus/master/send automation, free
   CC lanes, pitch bend.
5. Built-in EQ and limiter.
6. Transport cycle (loop) region.
7. Later: clock-time anchoring, warping, video, per-note expression,
   VCAs, analyzers.
