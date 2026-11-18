//! Every built-in sound: stable, calibrated, and quick enough.

use corinzia_ensemble::patches::{self, Model, PATCHES};
use corinzia_ensemble::voice::{self, tech, Voice};

const SR: f64 = 48000.0;

fn note(patch: usize, key: i16, vel: f32, secs: f32, hold: f32) -> Vec<f32> {
    let mut v = Voice::new(SR);
    v.start(patch, tech::ORD, 0, 0, 0, key, vel, Some(vel), 0, 7);
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
        v.render(&mut out[at..end], 0.0, trims);
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
        let measured = voice::measure(i, SR);
        // Measured with the trims applied: the target, within 0.5 dB.
        let t = patches::trims()[i];
        let target = patches::REFERENCE_RMS * 10f32.powf(p.level_db / 20.0);
        let db = 20.0 * (measured * t / target).log10();
        assert!(db.abs() < 0.5, "{}: {db} dB", p.name);
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
    assert!(s < if cfg!(debug_assertions) { 60.0 } else { 0.8 });
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

#[test]
fn instrument_names_choose_their_sounds() {
    let of = |n: &str| patches::PATCHES[patches::patch_for(patches::program_for(n, false, false).0, patches::program_for(n, false, false).1)].name;
    let orch = |n: &str| patches::PATCHES[patches::patch_for(patches::program_for(n, false, true).0, patches::program_for(n, false, true).1)].name;
    for (name, sound) in [
        ("Flute", "Flute"),
        ("Alto Flute", "Flute"),
        ("Harpsichord", "Harpsichord"),
        ("Harp", "Harp"),
        ("Lute", "Guitar"),
        ("Violin", "Violin"),
        ("Violins 1", "Violins"),
        ("Violoncello", "Cello"),
        ("Horn in F", "Horn"),
        ("Horns", "Horn"),
        ("English Horn", "Cor anglais"),
        ("Cornet", "Trumpet"),
        ("Bass Clarinet", "Bass clarinet"),
        ("Contrabassoon", "Contrabassoon"),
        ("Alto Saxophone", "Saxophone"),
        ("Tenor Trombone", "Trombone"),
        ("Soprano", "Voice (female)"),
        ("Tenor", "Voice (male)"),
        ("Choir", "Choir"),
        ("Timpani", "Timpani"),
        ("Piano", "Piano"),
        ("Glockenspiel", "Glockenspiel"),
    ] {
        assert_eq!(of(name), sound, "{name}");
    }
    assert_eq!(orch("Violin I"), "Violins");
    assert_eq!(orch("Solo Violin"), "Violin");
    assert_eq!(orch("Double Bass"), "Basses");
    assert_eq!(patches::PATCHES[patches::patch_for(0, patches::DRUM_BANK)].name, "Drum kit");
}

/// Chromatic runs like the University of Iowa recordings (one note a
/// second and a half, at pp, mf and ff), as WAV files for comparing
/// spectra: CORINZIA_RUNS=dir cargo test --release -p corinzia-ensemble write_runs -- --ignored
#[test]
#[ignore]
fn write_runs() {
    let Ok(dir) = std::env::var("CORINZIA_RUNS") else { return };
    let trims = patches::trims();
    for (patch, name, lo, hi) in [(5usize, "Violin", 55i16, 93i16), (6, "Viola", 48, 84), (7, "Cello", 36, 72), (8, "Bass", 28, 62), (0, "Violins", 55, 93), (2, "Cellos", 36, 72)] {
        for (level, vel) in [("pp", 0.26f32), ("mf", 0.62), ("ff", 0.87)] {
            let note = (1.5 * SR) as usize;
            let mut out = vec![0.0f32; note * (hi - lo + 2) as usize];
            for (k, key) in (lo..=hi).enumerate() {
                let mut v = Voice::new(SR);
                v.start(patch, tech::ORD, 0, 0, 0, key, vel, Some(vel), 0, k as u32);
                let start = k * note + note / 4;
                let len = note * 2 / 3;
                let mut at = 0;
                while at < note && start + at < out.len() {
                    let end = (at + 256).min(note).min(out.len() - start);
                    if at <= len && len < end {
                        v.release();
                    }
                    v.render(&mut out[start + at..start + end], 0.0, trims);
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
fn strings_have_the_measured_bodies_roll_off() {
    // A violin's A4 at mf: the harmonics around 6 kHz lie far under those
    // around 1 kHz, as in the recordings (about 27 dB); a cello's C3 falls
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
