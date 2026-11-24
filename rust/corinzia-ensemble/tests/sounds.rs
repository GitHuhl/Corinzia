//! Every built-in sound: stable, calibrated, and quick enough.

use corinzia_ensemble::patches::{self, Model, PATCHES};
use corinzia_ensemble::voice::{self, tech, Voice};

const SR: f64 = 48000.0;

/// Bowed strings play through their players: as many as the sound's own,
/// from `pool` (never another voice's: `id` is this voice's).
fn with_players(v: &mut Voice, pool: &mut voice::PlayerPool, id: u32) {
    if v.wants_players() {
        let mut slots = [0u16; voice::MAX_SECTION];
        let k = pool.take(id, PATCHES[v.patch].unison.max(1) as usize, |o, _| o != id, &mut slots);
        v.attach_players(&slots[..k], pool, true);
    }
}

fn pool() -> voice::PlayerPool {
    voice::PlayerPool::new(SR, 16)
}

fn note(patch: usize, key: i16, vel: f32, secs: f32, hold: f32) -> Vec<f32> {
    let mut v = Voice::new(SR);
    v.start(patch, tech::ORD, 0, 0, 0, key, vel, Some(vel), 0, 7);
    let mut pool = pool();
    with_players(&mut v, &mut pool, 0);
    let n = (secs * SR as f32) as usize;
    let mut out = vec![0.0; n];
    let off = (hold * SR as f32) as usize;
    let trims = patches::trims();
    let mut at = 0;
    while at < n {
        let end = (at + 256).min(n);
        if at <= off && off < end {
            v.release();
        }
        v.render_with(&mut out[at..end], 0.0, trims, &mut pool);
        at = end;
    }
    out
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn every_sound_is_stable_and_ends() {
    for (i, p) in PATCHES.iter().enumerate() {
        for key in [p.ref_key as i16 - 12, p.ref_key as i16, p.ref_key as i16 + 12] {
            let key = if p.model == Model::Kit { 38 } else { key };
            for vel in [0.16f32, 0.62, 1.0] {
                let out = note(i, key, vel, 3.0, 1.0);
                assert!(out.iter().all(|x| x.is_finite()), "{} {key} {vel}: not finite", p.name);
                let peak = out.iter().fold(0.0f32, |a, x| a.max(x.abs()));
                assert!(peak < 1.0, "{} {key} {vel}: peak {peak}", p.name);
                assert!(rms(&out[..24000]) > 1e-4, "{} {key} {vel}: silent", p.name);
            }
        }
    }
}

#[test]
fn sounds_are_calibrated_to_their_orchestral_level() {
    for (i, p) in PATCHES.iter().enumerate() {
        // Bowed strings through their players (who calibrate themselves).
        let bowed = p.model == Model::Bowed;
        // (A section's players sum differently on each note, as a real
        // section's do: over thirteen notes about its reference key.)
        let measured = if bowed && p.unison > 1 {
            let keys = (p.ref_key as i16 - 6)..=(p.ref_key as i16 + 6);
            let power: f32 = keys.clone().map(|k| voice::measure_note(i, SR, Some(&mut pool()), tech::ORD, k).powi(2)).sum();
            (power / keys.count() as f32).sqrt()
        } else if bowed {
            voice::measure_with(i, SR, Some(&mut pool()), tech::ORD)
        } else {
            voice::measure(i, SR)
        };
        // Measured with the trims applied: the target, within 0.5 dB.
        let t = if bowed { 1.0 } else { patches::trims()[i] };
        // (A section's sum keeps some spread even over thirteen notes.)
        let tolerance = if bowed && p.unison > 1 { 1.5 } else { 0.5 };
        let target = patches::REFERENCE_RMS * 10f32.powf(p.level_db / 20.0);
        let db = 20.0 * (measured * t / target).log10();
        assert!(db.abs() < tolerance, "{}: {db} dB", p.name);
    }
    // A trumpet at ff is far louder than a flute at ff.
    let tpt = rms(&note(23, 72, 1.0, 1.0, 1.0)[14400..38400]);
    let fl = rms(&note(13, 74, 1.0, 1.0, 1.0)[14400..38400]);
    assert!(20.0 * (tpt / fl).log10() > 8.0, "{tpt} {fl}");
    // A clarinet fades further than an oboe.
    let range = |p: usize, k: i16| 20.0 * (rms(&note(p, k, 1.0, 1.0, 1.0)[14400..38400]) / rms(&note(p, k, 0.16, 1.0, 1.0)[14400..38400])).log10();
    assert!(range(16, 65) > range(14, 69) + 8.0);
}

#[test]
fn a_clarinet_lacks_even_harmonics_in_the_chalumeau() {
    let out = note(16, 55, 0.62, 1.0, 1.0);
    let f = 440.0 * 2f32.powf((55.0 - 69.0) / 12.0);
    let h = |n: f32| {
        let w = std::f32::consts::TAU * f * n / SR as f32;
        let (mut re, mut im) = (0.0, 0.0);
        for (i, x) in out[19200..43200].iter().enumerate() {
            re += x * (w * i as f32).cos();
            im += x * (w * i as f32).sin();
        }
        (re * re + im * im).sqrt()
    };
    assert!(h(3.0) > h(2.0) * 3.0 && h(5.0) > h(4.0) * 2.0, "{} {} {} {}", h(2.0), h(3.0), h(4.0), h(5.0));
}

#[test]
fn renders_faster_than_real_time() {
    let t = std::time::Instant::now();
    // 32 string and brass notes for a second.
    for k in 0..32 {
        let _ = note(if k % 2 == 0 { 0 } else { 23 }, 55 + k as i16, 0.62, 1.0, 1.0);
    }
    let s = t.elapsed().as_secs_f32();
    eprintln!("32 voice-seconds in {s:.2} s");
    // Well inside real time (32 s), with room for a busy machine; the
    // crate is optimised in every build (see the workspace's profiles).
    assert!(s < 3.2, "{s} s");
}

#[test]
#[ignore]
fn debug_peaks() {
    for (i, p) in PATCHES.iter().enumerate() {
        let key = if p.model == Model::Kit { 38 } else { p.ref_key as i16 };
        for vel in [0.62f32, 1.0] {
            let out = note(i, key - 12, vel, 1.5, 1.0);
            let peak = |a: usize, b: usize| out[a..b].iter().fold(0.0f32, |m, x| m.max(x.abs()));
            eprintln!("{:22} v{vel}: attack peak {:.3}  sustain peak {:.3} rms {:.4}  trim {:.3}", p.name, peak(0, 4800), peak(14400, 38400), rms(&out[14400..38400]), patches::trims()[i]);
        }
    }
}

/// Chromatic runs like the University of Iowa recordings (one note a
/// second and a half, at pp, mf and ff), as WAV files for comparing
/// spectra: CORINZIA_RUNS=dir cargo test --release -p corinzia-ensemble write_runs -- --ignored
#[test]
#[ignore]
fn write_runs() {
    let Ok(dir) = std::env::var("CORINZIA_RUNS") else { return };
    let trims = patches::trims();
    let which = std::env::var("CORINZIA_RUNS_SET").unwrap_or_default();
    let strings = [(5usize, "Violin", 55i16, 93i16), (6, "Viola", 48, 84), (7, "Cello", 36, 72), (8, "Bass", 28, 62), (0, "Violins", 55, 93), (2, "Cellos", 36, 72)];
    let winds = [(13usize, "Flute", 60i16, 96i16), (14, "Oboe", 58, 91), (16, "BbClar", 50, 91), (17, "BassClarinet", 34, 77), (18, "Bassoon", 34, 74), (20, "AltoSax", 49, 80), (22, "Horn", 34, 77), (23, "Trumpet", 52, 86), (24, "TenorTrombone", 40, 72), (25, "Tuba", 26, 60)];
    let set: &[(usize, &str, i16, i16)] = if which == "winds" { &winds } else { &strings };
    for &(patch, name, lo, hi) in set {
        for (level, vel) in [("pp", 0.26f32), ("mf", 0.62), ("ff", 0.87)] {
            let note = (1.5 * SR) as usize;
            let mut out = vec![0.0f32; note * (hi - lo + 2) as usize];
            for (k, key) in (lo..=hi).enumerate() {
                let mut v = Voice::new(SR);
                v.start(patch, tech::ORD, 0, 0, 0, key, vel, Some(vel), 0, k as u32);
                let mut pool = pool();
                with_players(&mut v, &mut pool, 0);
                let start = k * note + note / 4;
                let len = note * 2 / 3;
                let mut at = 0;
                while at < note && start + at < out.len() {
                    let end = (at + 256).min(note).min(out.len() - start);
                    if at <= len && len < end {
                        v.release();
                    }
                    v.render_with(&mut out[start + at..start + end], 0.0, trims, &mut pool);
                    at = end;
                }
            }
            let path = format!("{dir}/{name}.arco.{level}.{}{}.synth.wav", note_name(lo), note_name(hi));
            write_wav(&path, &out, SR as u32);
        }
    }
}

fn note_name(key: i16) -> String {
    const N: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
    format!("{}{}", N[(key % 12) as usize], key / 12 - 1)
}

fn write_wav(path: &str, x: &[f32], sr: u32) {
    let mut b = Vec::with_capacity(44 + x.len() * 2);
    let data = (x.len() * 2) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&sr.to_le_bytes());
    b.extend_from_slice(&(sr * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    // Normalised: only the spectrum is compared.
    let peak = x.iter().fold(1e-9f32, |m, v| m.max(v.abs()));
    for v in x {
        b.extend_from_slice(&((v / peak * 0.9 * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

/// The level (dB) of a frequency band in a stretch of samples.
fn band_db(x: &[f32], lo: f32, hi: f32) -> f32 {
    let n = x.len();
    let mut sum = 0.0f64;
    let k0 = (lo * n as f32 / SR as f32) as usize;
    let k1 = (hi * n as f32 / SR as f32) as usize;
    for k in k0..=k1 {
        let w = std::f64::consts::TAU * k as f64 / n as f64;
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let win = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            re += *v as f64 * win * (w * i as f64).cos();
            im += *v as f64 * win * (w * i as f64).sin();
        }
        sum += re * re + im * im;
    }
    (10.0 * (sum / (k1 - k0 + 1) as f64).log10()) as f32
}

#[test]
fn strings_roll_off_as_real_bodies_do() {
    // A violin's A4 at mf: the harmonics around 6 kHz lie far under those
    // around 1 kHz, as in recordings (about 27 dB); a cello's C3 falls
    // from its low register.
    let v = note(5, 69, 0.62, 1.0, 1.0);
    let x = &v[24000..28096];
    let drop = band_db(x, 800.0, 1300.0) - band_db(x, 5500.0, 7000.0);
    assert!(drop > 18.0, "violin: {drop} dB");
    let c = note(7, 48, 0.62, 1.0, 1.0);
    let y = &c[24000..28096];
    let fall = band_db(y, 120.0, 300.0) - band_db(y, 2000.0, 3000.0);
    assert!(fall > 20.0, "cello: {fall} dB");
    // A section is not one player: the same note differs.
    let s = note(0, 69, 0.62, 1.0, 1.0);
    assert!((rms(&s[24000..28096]) - rms(x)).abs() > 1e-5 || s[24000..24100] != v[24000..24100]);
}

/// Isolated events for click hunting:
/// CORINZIA_RUNS=dir cargo test --release -p corinzia-ensemble write_events -- --ignored
#[test]
#[ignore]
fn write_events() {
    let Ok(dir) = std::env::var("CORINZIA_RUNS") else { return };
    let trims = patches::trims();
    for (patch, name) in [(5usize, "violin"), (0, "violins"), (13, "flute"), (23, "trumpet"), (16, "clarinet")] {
        let key = patches::PATCHES[patch].ref_key as i16;
        // 1: a crescendo in CC steps every 25 ms; 2: a slur (legato_to);
        // 3: a note off then a new note on the same voice slot.
        for event in ["cresc", "slur", "repeat"] {
            let mut v = Voice::new(SR);
            let mut w = Voice::new(SR);
            v.start(patch, tech::ORD, 0, 0, 0, key, 0.4, Some(0.4), 0, 1);
            let mut pool = pool();
            with_players(&mut v, &mut pool, 0);
            let n = (3.0 * SR) as usize;
            let mut out = vec![0.0f32; n];
            let mut at = 0;
            while at < n {
                let t = at as f32 / SR as f32;
                match event {
                    "cresc" if (1.0..2.0).contains(&t) => v.dyn_target = 0.4 + 0.5 * (((t - 1.0) / 0.025).floor() * 0.025),
                    "slur" if (t - 1.0).abs() < 0.003 => v.legato_to(1, key + 2, 0.5, 1),
                    "slur" if (t - 2.0).abs() < 0.003 => v.legato_to(2, key - 3, 0.5, 2),
                    "repeat" if (t - 1.0).abs() < 0.003 => v.release(),
                    "repeat" if (t - 1.06).abs() < 0.003 => {
                        w.start(patch, tech::ORD, 3, 0, 0, key + 2, 0.5, Some(0.5), 3, 4);
                        with_players(&mut w, &mut pool, 1);
                    }
                    _ => {}
                }
                let end = (at + 97).min(n);
                v.render_with(&mut out[at..end], 0.0, trims, &mut pool);
                w.render_with(&mut out[at..end], 0.0, trims, &mut pool);
                at = end;
            }
            write_wav(&format!("{dir}/{name}-{event}.wav"), &out, SR as u32);
        }
    }
}

#[test]
fn split_blocks_play_like_whole_ones() {
    // The host splits its blocks at every event: a note in pieces of odd
    // sizes is the same note (no silent starts of a piece, no clicks).
    let trims = patches::trims();
    for patch in [0usize, 5, 13, 16, 23, 28, 32] {
        let key = patches::PATCHES[patch].ref_key as i16;
        let render = |sizes: &[usize]| {
            let mut v = Voice::new(SR);
            v.start(patch, tech::ORD, 0, 0, 0, key, 0.6, Some(0.6), 0, 3);
            let mut pool = pool();
            with_players(&mut v, &mut pool, 0);
            let mut out = vec![0.0f32; 24000];
            let (mut at, mut k) = (0, 0);
            while at < out.len() {
                let end = (at + sizes[k % sizes.len()]).min(out.len());
                v.render_with(&mut out[at..end], 0.0, trims, &mut pool);
                at = end;
                k += 1;
            }
            out
        };
        let whole = render(&[256]);
        let split = render(&[1, 7, 13, 3, 61, 2, 29]);
        let (w, s) = (rms(&whole[4800..]), rms(&split[4800..]));
        // (the measured instruments' intonation wanders at random, a
        // section's players each: within a decibel)
        let wanders = PATCHES[patch].unison > 1 || patches::spectrum(patch).is_some_and(|x| x.body.is_some());
        assert!((20.0 * (s / w).log10()).abs() < if wanders { 1.0 } else { 0.5 }, "{}: {w} vs {s}", PATCHES[patch].name);
        // No sample-to-sample jumps far beyond the sound's own.
        let jump = |x: &[f32]| x.windows(2).map(|p| (p[1] - p[0]).abs()).fold(0.0f32, f32::max);
        assert!(jump(&split[4800..]) < 2.0 * jump(&whole[4800..]) + 1e-4, "{}", PATCHES[patch].name);
    }
}

/// A note with an articulation: velocity `vel`, settling at `dynamic`.
fn articulated(patch: usize, key: i16, vel: f32, dynamic: f32, code: u8, hold: f32) -> Vec<f32> {
    let mut v = Voice::new(SR);
    v.start(patch, tech::ORD, 0, 0, 0, key, vel, Some(dynamic), 0, 7);
    v.articulate(code, true);
    let mut pool = pool();
    with_players(&mut v, &mut pool, 0);
    let n = (1.2 * SR as f32) as usize;
    let mut out = vec![0.0; n];
    let off = (hold * SR as f32) as usize;
    let trims = patches::trims();
    let mut at = 0;
    while at < n {
        let end = (at + 64).min(n);
        if at <= off && off < end {
            v.release();
        }
        v.render_with(&mut out[at..end], 0.0, trims, &mut pool);
        at = end;
    }
    out
}

/// The time (s) the 5 ms level takes to reach 90 % of its 0.3-0.4 s level.
fn rise_time(x: &[f32]) -> f32 {
    let w = (0.005 * SR) as usize;
    let steady = rms(&x[(0.3 * SR) as usize..(0.4 * SR) as usize]);
    (0..x.len() / w).find(|&k| rms(&x[k * w..(k + 1) * w]) > 0.9 * steady).map_or(1.0, |k| (k * w) as f32 / SR as f32)
}

#[test]
fn articulations_change_how_notes_speak() {
    use voice::articulation as a;
    for (patch, key) in [(5usize, 69i16), (13, 74), (23, 72), (0, 69)] {
        let name = PATCHES[patch].name;
        // Staccato speaks quicker than a plain note.
        let plain = articulated(patch, key, 0.62, 0.62, a::NORMAL, 1.0);
        let short = articulated(patch, key, 0.62, 0.62, a::STACCATO, 1.0);
        assert!(rise_time(&short) < rise_time(&plain) * 0.8, "{name}: {} vs {}", rise_time(&short), rise_time(&plain));
        // A marcato's start is brighter than a plain one's.
        let marc = articulated(patch, key, 0.62, 0.62, a::MARCATO, 1.0);
        let bright = |x: &[f32]| {
            let s = &x[(0.03 * SR) as usize..(0.03 * SR) as usize + 2048];
            let d: Vec<f32> = s.windows(2).map(|w| w[1] - w[0]).collect();
            rms(&d) / rms(s) // the slope's share: the high harmonics
        };
        if PATCHES[patch].model == Model::Flute {
            // A flute's start is mostly breath: a marcato's chiff is louder.
            let start = |x: &[f32]| rms(&x[..(0.04 * SR) as usize]);
            assert!(start(&marc) > start(&plain) * 1.3, "{name}: {} vs {}", start(&marc), start(&plain));
        } else {
            // (A bow's bite is mostly pressure: its tone brightens less than
            // a reed's or lips'.)
            let more = if PATCHES[patch].model == Model::Bowed { 1.1 } else { 1.15 };
            assert!(bright(&marc) > bright(&plain) * more, "{name}: {} vs {}", bright(&marc), bright(&plain));
        }
        // A staccato's release is shorter: less left 80 ms after the end.
        let tail = |x: &[f32]| rms(&x[(0.58 * SR) as usize..(0.62 * SR) as usize]);
        let plain_short = articulated(patch, key, 0.62, 0.62, a::NORMAL, 0.5);
        assert!(tail(&short_held(patch, key)) < tail(&plain_short) * 0.8, "{name}");
    }
    // An fp: forte as it speaks, piano soon after.
    for (patch, key) in [(5usize, 69i16), (13, 74), (23, 72)] {
        let fp = articulated(patch, key, 0.75, 0.38, a::NORMAL, 1.0);
        let w = (0.01 * SR) as usize;
        let early = (0..30).map(|k| rms(&fp[k * w..(k + 1) * w])).fold(0.0f32, f32::max);
        let late = rms(&fp[(0.6 * SR) as usize..(0.7 * SR) as usize]);
        assert!(20.0 * (early / late).log10() > 6.0, "{}: {early} {late}", PATCHES[patch].name);
    }
}

fn short_held(patch: usize, key: i16) -> Vec<f32> {
    articulated(patch, key, 0.62, 0.62, voice::articulation::STACCATO, 0.5)
}
