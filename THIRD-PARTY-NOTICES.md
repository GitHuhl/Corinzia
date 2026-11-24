# Third-party code in Corinzia

Corinzia is licensed under the GNU Affero General Public License, version 3
or later (see `LICENSE`). Some of its code is adapted from other free
software projects. Those parts keep their original copyright and licence,
listed here, and say so in the files that contain them.

| Corinzia file | Adapted from | Licence |
|---|---|---|
| `rust/corinzia-audio/src/analysis.rs` (`onset_strength`, `log_bands`) | SoundCraft, `crates/dsp/src/offline.rs` (`detect_transients`, `log_bands`), https://github.com/storytold/soundcraft | MIT |
| `rust/corinzia-ensemble/src/effects.rs` (the compressor's and gate's gain computers and ballistics) | SoundCraft, `crates/dsp/src/plugins/dynamics.rs` | MIT |
| `rust/corinzia-ensemble/src/bowed.rs` (the bowed string instrument: string waveguides, bow friction, body modes) | Strings, `src/dsp/PhysicalModel.cpp`, https://github.com/kenotron-ms/strings-vst (whose bow-reflection equation and waveguide topology are informed by the Synthesis ToolKit's Bowed/BowTable) | MIT |
| `rust/corinzia-audio/src/loudness.rs` (K-weighting design, true-peak interpolator) | SoundCraft, `crates/dsp/src/meter.rs` | MIT |
| `rust/corinzia-layout/src/beam.rs` (beam quanting with collisions and knees, stems' ideal and shortest lengths) | GNU LilyPond, `lily/beam-quanting.cc`, `lily/stem.cc`, `scm/define-grobs.scm` | GPL-3.0-or-later |
| `rust/corinzia-layout/src/slur.rs` (slur placement and scoring) | GNU LilyPond, `lily/slur-scoring.cc`, `lily/slur-configuration.cc`, `lily/bezier-bow.cc`, `scm/layout-slur.scm`, https://lilypond.org | GPL-3.0-or-later |
| `rust/corinzia-layout/src/layout/spacing.rs` (duration spacing below the shortest note) | GNU LilyPond, `lily/spacing-options.cc`, `scm/define-grobs.scm` | GPL-3.0-or-later |
| `rust/corinzia-layout/src/layout/notes.rs` (chords' accidentals, `place_accidentals`; rests clear of other voices' notes and rests; chord dots, `dot_rows`) | GNU LilyPond, `lily/accidental-placement.cc`, `lily/rest-collision.cc`, `lily/dot-configuration.cc`, `lily/dot-column.cc` | GPL-3.0-or-later |
| `rust/corinzia-layout/src/layout/ties.rs` (ties' shape, placement, directions, single-tie variations, dot collisions and chords' tie columns, `tie_shape` and `draw_ties`) | GNU LilyPond, `lily/tie-formatting-problem.cc`, `lily/tie-configuration.cc`, `lily/tie.cc` | GPL-3.0-or-later |
| `rust/corinzia-layout/src/layout/scripts.rs` (articulation placement) | GNU LilyPond, `scm/script.scm`, `lily/side-position-interface.cc` | GPL-3.0-or-later |
| `rust/corinzia-layout/src/layout/tuplets.rs` (tuplet brackets, `draw_tuplets`) | GNU LilyPond, `lily/tuplet-bracket.cc` | GPL-3.0-or-later |
| `rust/corinzia-layout/src/layout/dynamics.rs` (dynamics' and hairpins' padding and alignment; hairpin ends) | GNU LilyPond, `lily/hairpin.cc`, `lily/side-position-interface.cc`, `scm/define-grobs.scm` | GPL-3.0-or-later |
| `rust/corinzia-layout/src/layout/pages.rs` (vertical spacing and page breaking in `layout_pages`) | GNU LilyPond, `ly/paper-defaults-init.ly` | GPL-3.0-or-later |

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

## Strings

https://github.com/kenotron-ms/strings-vst (its licence names no copyright holder; the
Strings authors).

MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above permission notice shall be included in all copies or substantial
portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.

## The Synthesis ToolKit in C++ (STK)

Credited by Strings, above; no STK code is in Corinzia.

Copyright (c) 1995-2023 Perry R. Cook and Gary P. Scavone

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

Any person wishing to distribute modifications to the Software is
asked to send the modifications to the original developer so that they
can be incorporated into the canonical version.  This is, however, not
a binding provision of this license.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR
ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF
CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION
WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
