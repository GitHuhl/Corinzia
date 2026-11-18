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
