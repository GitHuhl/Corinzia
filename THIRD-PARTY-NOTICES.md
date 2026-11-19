# Third-party code in Corinzia

Corinzia is licensed under the GNU Affero General Public License, version 3
or later (see `LICENSE`). Some of its code is adapted from other free
software projects. Those parts keep their original copyright and licence,
listed here, and say so in the files that contain them.

| Corinzia file | Adapted from | Licence |
|---|---|---|
| `rust/corinzia-audio/src/analysis.rs` (`onset_strength`, `log_bands`) | SoundCraft, `crates/dsp/src/offline.rs` (`detect_transients`, `log_bands`), https://github.com/storytold/soundcraft | MIT |
| `rust/corinzia-ensemble/src/effects.rs` (the compressor's and gate's gain computers and ballistics) | SoundCraft, `crates/dsp/src/plugins/dynamics.rs` | MIT |
| `rust/corinzia-audio/src/loudness.rs` (K-weighting design, true-peak interpolator) | SoundCraft, `crates/dsp/src/meter.rs` | MIT |
| `rust/corinzia-layout/src/beam.rs` (beam quanting, stems' ideal and shortest lengths) | GNU LilyPond, `lily/beam-quanting.cc`, `lily/stem.cc`, `scm/define-grobs.scm` | GPL-3.0-or-later |
| `rust/corinzia-layout/src/slur.rs` (slur placement and scoring) | GNU LilyPond, `lily/slur-scoring.cc`, `lily/slur-configuration.cc`, `lily/bezier-bow.cc`, `scm/layout-slur.scm`, https://lilypond.org | GPL-3.0-or-later |
| `rust/corinzia-layout/src/layout.rs` (dynamics' and hairpins' padding; single ties' shape and placement, `tie_shape` and `draw_ties`) | GNU LilyPond, `scm/define-grobs.scm` (DynamicLineSpanner, Tie), `lily/tie-formatting-problem.cc`, `lily/tie-configuration.cc` | GPL-3.0-or-later |

## GNU LilyPond

Copyright (C) 1996–2026 Han-Wen Nienhuys, Jan Nieuwenhuizen and the
LilyPond authors (https://lilypond.org).

GNU LilyPond is free software: you can redistribute it and/or modify it
under the terms of the GNU General Public License as published by the Free
Software Foundation, either version 3 of the License, or (at your option)
any later version. The code adapted from it stays under that licence; its
text is in `licenses/GPL-3.0.txt`. Section 13 of both the GNU GPL v3 and
the GNU AGPL v3 allows the two to be combined in one program, as here.

## SoundCraft

MIT License

Copyright (c) 2026 ArtCraft Team and the SoundCraft contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
