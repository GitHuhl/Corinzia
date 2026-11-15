# Developing Corinzia

Corinzia is written in Rust. The score is engraved by its own engraver
(Bravura/SMuFL, and Neanes/SBMuFL for Byzantine chant) and drawn on the GPU
with iced; playback hosts CLAP and VST3 instruments in a lock-free realtime
engine, rendering instruments in parallel on every core, that outputs
through PipeWire (WASAPI on Windows).

The design is described in [ARCHITECTURE.md](ARCHITECTURE.md); how
Corinzia compares with DAWproject, in [DAW-PARITY.md](DAW-PARITY.md).

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

The README's screenshots are remade by `scripts/readme-screenshots.sh`
(headless, with a private PipeWire session), from public-domain scores in
the music21 corpus and the demo project.

## Layout

| Path | What |
|---|---|
| `rust/corinzia-app` | the application (iced): score canvas, editor, panels, playback, Mix tab, files |
| `rust/corinzia-byz` | Byzantine chant (Chrysanthine notation): neumes, modes, scales in moria, the melody a chant spells |
| `rust/corinzia-edit` | pure editing operations: note input, ranges, structure, popover parsing, pitch and interval names |
| `rust/corinzia-model` | project model and `.project` storage (SQLite, `../schema/project.sql`) |
| `rust/corinzia-layout` | the engraver: display lists, page layout, PDF |
| `rust/corinzia-perform` | performance: repeats, ties, ornaments, dynamics; tempo maps; export documents |
| `rust/corinzia-audio` | CLAP and VST3 host, plugin editors, realtime engine, PipeWire output |
| `rust/corinzia-sine` | Corinzia Sine, the built-in CLAP instrument |
| `rust/corinzia-export` | MIDI and MusicXML writers |
| `rust/corinzia-score` | exact fractions, note values, the engraver's input types |
| `assets/fonts/bravura` | Bravura (SIL OFL) and its SMuFL metadata |
| `assets/fonts/neanes` | Neanes (SIL OFL), a Byzantine notation font, and its SBMuFL metadata |
| `scripts/` | headless screenshots, a private PipeWire session for the live audio test |
