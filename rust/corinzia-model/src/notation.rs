//! A movement as the engraver's and exporters' input
//! (`corinzia_score::score::Score`): staff names and clefs from each lane's
//! first instrument, instrument changes written as text.

use crate::model::*;
use corinzia_score::score as sc;

pub fn score(project: &Project, m: &Movement) -> sc::Score {
    let parts = m
        .lanes
        .iter()
        .map(|lane| {
            let first = lane.changes.first().and_then(|c| project.instrument(c.instrument));
            let name = first.map(|i| i.name.clone()).or_else(|| project.player(lane.player).map(|p| p.name.clone())).unwrap_or_default();
            let mut texts: Vec<sc::Text> = lane
                .texts
                .iter()
                .map(|t| sc::Text { at: t.at, text: t.text.clone(), technique: t.kind == TextKind::Technique })
                .collect();
            for c in lane.changes.iter().skip(1) {
                if let Some(i) = project.instrument(c.instrument) {
                    texts.push(sc::Text { at: c.at, text: i.name.clone(), technique: true });
                }
            }
            sc::Part {
                name,
                abbr: first.map(|i| i.engraver_name.clone()).unwrap_or_default(),
                clef: first.map(|i| i.clef.clone()).unwrap_or_else(|| "treble".into()),
                clefs: lane.clefs.iter().map(|c| sc::ClefChange { at: c.at, clef: c.clef.clone() }).collect(),
                texts,
                chords: lane.chords.iter().map(|c| sc::ChordSymbol { at: c.at, text: c.text.clone(), id: c.id }).collect(),
                voices: lane
                    .voices
                    .iter()
                    .map(|v| sc::Voice {
                        index: v.index,
                        events: v
                            .events
                            .iter()
                            .map(|e| sc::Event {
                                id: e.id,
                                start: e.start,
                                dur: e.duration,
                                pitches: if e.is_rest() {
                                    vec![]
                                } else {
                                    e.heads
                                        .iter()
                                        .map(|h| sc::Pitch { id: h.id, step: h.pitch.step as u8, alter: h.pitch.alter, octave: h.pitch.octave, tie: h.tie })
                                        .collect()
                                },
                                art: e.articulations,
                                dynamic: e.dynamic.map(|d| DYNAMICS[(d as usize).min(7)].to_string()),
                                spans: e.spans,
                                lyrics: e.lyrics.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            }
        })
        .collect();
    sc::Score {
        title: m.title.clone(),
        length: m.length,
        tempo: m.tempo.iter().map(|t| sc::Tempo { at: t.at, bpm: t.bpm, text: t.text.clone() }).collect(),
        meter: m.meter.iter().map(|t| sc::Meter { at: t.at, num: t.num, den: t.den }).collect(),
        keys: m.keys.iter().map(|k| sc::Key { at: k.at, fifths: k.fifths, minor: k.minor }).collect(),
        barlines: m
            .barlines
            .iter()
            .map(|b| sc::Barline {
                at: b.at,
                kind: match b.kind {
                    BarKind::Double => sc::BarKind::Double,
                    BarKind::Final => sc::BarKind::Final,
                    BarKind::RepeatStart => sc::BarKind::RepeatStart,
                    BarKind::RepeatEnd => sc::BarKind::RepeatEnd,
                    BarKind::RepeatBoth => sc::BarKind::RepeatBoth,
                },
            })
            .collect(),
        endings: m
            .endings
            .iter()
            .map(|e| sc::Ending { from: e.from, to: e.to, numbers: (1..=8u8).filter(|n| e.passes & (1 << (n - 1)) != 0).collect() })
            .collect(),
        marks: m.rehearsal.iter().map(|r| r.at).collect(),
        parts,
    }
}
