//! Measured spectral envelopes of the bowed strings (generated; do not edit).
//!
//! From the University of Iowa Musical Instrument Samples (2012 strings,
//! arco, pp/mf/ff chromatic runs): each note's harmonic levels in its
//! steady part, fitted as one curve over absolute frequency per
//! instrument and dynamic (dB, 0 at 1 kHz), on knots every sixth of an
//! octave from 50 Hz. The viola's is the violin's a third of an octave
//! lower, with the cello's shape under 400 Hz.

pub const KNOT0: f32 = 50.0;
pub const PER_OCTAVE: f32 = 6.0;
pub const KNOTS: usize = 51;

/// [instrument][pp, mf, ff][knot]: violin, viola, cello, double bass.
pub static BODY: [[[f32; KNOTS]; 3]; 4] = [
    // Violin
    [
        [8.4, 8.4, 8.4, 8.4, 8.4, 8.4, 8.4, 8.4, 8.4, 8.4, 8.4, 8.4, 8.4, 12.8, 16.7, 19.8, 21.5, 21.6, 19.9, 17.0, 14.6, 12.0, 8.4, 7.8, 7.9, 3.2, -0.2, -1.6, -2.3, -1.6, -3.4, -3.1, -0.8, -2.4, -8.1, -10.9, -10.5, -8.6, -12.5, -17.5, -26.6, -29.4, -35.7, -39.5, -42.1, -44.6, -46.5, -47.9, -48.5, -49.3, -51.1],
        [3.1, 3.1, 3.1, 3.1, 3.1, 3.1, 3.1, 3.1, 3.1, 3.1, 3.1, 3.1, 3.1, 10.7, 17.8, 22.7, 23.7, 22.2, 19.8, 16.2, 13.6, 11.3, 7.0, 5.0, 5.8, 2.4, -0.2, 1.1, -0.1, -2.0, -2.0, 0.1, 4.0, 1.0, -5.9, -6.8, -5.7, -5.5, -7.6, -11.1, -20.2, -21.0, -27.1, -31.9, -36.8, -39.3, -43.2, -44.8, -45.8, -48.6, -50.3],
        [24.8, 24.8, 24.8, 24.8, 24.8, 24.8, 24.8, 24.8, 24.8, 24.8, 24.8, 24.8, 24.8, 23.2, 24.5, 27.9, 29.6, 29.2, 26.6, 21.2, 16.6, 14.4, 9.6, 8.0, 9.1, 4.5, -0.3, 1.8, 1.1, -2.6, -3.8, -2.6, -1.6, -3.1, -7.8, -8.0, -7.7, -9.2, -11.0, -13.3, -18.4, -20.1, -22.6, -25.8, -28.8, -31.2, -33.2, -34.1, -37.0, -39.7, -42.1],
    ],
    // Viola
    [
        [25.4, 25.4, 25.4, 25.4, 25.4, 31.0, 34.1, 33.6, 32.4, 31.4, 30.7, 29.8, 27.4, 23.6, 21.0, 20.2, 20.3, 17.9, 14.6, 12.1, 8.5, 7.8, 7.9, 3.3, -0.1, -1.6, -2.3, -1.6, -3.4, -3.1, -0.8, -2.4, -8.0, -10.8, -10.5, -8.6, -12.4, -17.4, -26.4, -29.3, -35.6, -39.4, -42.0, -44.6, -46.5, -47.9, -48.5, -49.3, -51.1, -51.1, -51.1],
        [36.3, 36.3, 36.3, 36.3, 36.3, 36.3, 36.3, 32.4, 29.5, 28.0, 27.4, 27.0, 25.6, 22.1, 18.6, 17.5, 17.8, 16.1, 13.7, 11.3, 7.1, 5.0, 5.8, 2.5, -0.1, 1.1, -0.1, -2.0, -2.0, 0.1, 3.9, 1.1, -5.8, -6.8, -5.7, -5.5, -7.6, -11.0, -20.0, -21.0, -27.0, -31.8, -36.7, -39.3, -43.1, -44.8, -45.8, -48.5, -50.3, -50.3, -50.3],
        [16.8, 16.8, 16.8, 20.1, 23.3, 26.6, 29.9, 33.1, 36.4, 34.0, 31.6, 29.2, 26.8, 23.0, 20.8, 19.2, 19.9, 19.5, 16.7, 14.4, 9.7, 8.0, 9.1, 4.6, -0.2, 1.8, 1.1, -2.5, -3.8, -2.6, -1.6, -3.1, -7.7, -8.0, -7.7, -9.2, -11.0, -13.3, -18.3, -20.1, -22.6, -25.7, -28.7, -31.2, -33.2, -34.1, -36.9, -39.6, -42.1, -42.1, -42.1],
    ],
    // Cello
    [
        [28.4, 28.4, 28.4, 28.4, 28.4, 34.0, 37.1, 36.6, 35.4, 34.4, 33.7, 32.8, 30.4, 26.6, 24.0, 23.2, 23.3, 21.7, 17.6, 13.5, 13.0, 12.4, 8.9, 7.5, 5.1, 2.5, -0.2, -1.3, -4.4, -7.2, -7.5, -13.5, -14.9, -13.6, -12.7, -19.5, -22.6, -24.3, -26.1, -28.0, -28.8, -30.2, -31.1, -31.9, -32.3, -33.3, -34.1, -35.4, -36.6, -37.7, -38.9],
        [36.2, 36.2, 36.2, 36.2, 36.2, 36.2, 36.2, 32.3, 29.4, 27.9, 27.3, 26.9, 25.5, 22.0, 18.5, 17.4, 17.4, 15.9, 13.6, 11.6, 10.6, 9.5, 7.8, 6.9, 3.6, 1.2, -0.1, -2.1, -5.2, -8.4, -8.3, -12.7, -13.6, -12.8, -14.0, -21.0, -23.9, -26.5, -29.9, -33.0, -34.0, -35.7, -36.8, -39.2, -38.8, -41.0, -41.7, -45.7, -48.2, -50.6, -52.4],
        [17.4, 17.4, 17.4, 20.7, 23.9, 27.2, 30.5, 33.7, 37.0, 34.6, 32.2, 29.8, 27.4, 23.6, 21.4, 19.8, 19.2, 18.3, 17.3, 15.3, 13.7, 12.2, 10.4, 6.7, 3.4, 1.1, -0.1, -1.2, -3.9, -6.3, -7.9, -11.5, -12.4, -14.0, -15.1, -16.9, -18.8, -21.4, -25.9, -29.6, -31.2, -32.6, -33.8, -36.0, -35.7, -36.6, -38.6, -41.2, -43.3, -45.1, -46.6],
    ],
    // Bass
    [
        [16.8, 31.5, 42.1, 45.5, 43.4, 40.0, 36.7, 35.9, 35.6, 33.5, 31.6, 31.6, 28.0, 22.4, 17.8, 15.1, 14.3, 15.5, 18.4, 15.7, 10.4, 10.3, 4.9, 2.2, 2.3, 3.0, -0.2, 5.3, 3.0, -4.2, -5.7, -9.1, -13.4, -15.2, -19.1, -20.0, -25.9, -30.7, -29.6, -32.2, -38.3, -38.9, -39.2, -42.8, -44.1, -47.0, -50.3, -52.6, -54.1, -55.5, -55.5],
        [19.4, 34.0, 44.1, 47.6, 46.1, 42.6, 39.2, 37.6, 37.1, 35.1, 33.3, 33.2, 28.9, 22.8, 17.9, 15.9, 15.4, 16.9, 19.3, 15.7, 10.4, 10.2, 6.1, 3.0, 2.6, 2.5, -0.2, 5.2, 2.3, -3.6, -6.0, -7.7, -12.5, -15.8, -20.1, -19.8, -25.5, -31.3, -29.6, -32.4, -38.5, -38.3, -37.0, -41.5, -43.7, -45.9, -48.8, -51.0, -52.8, -54.4, -54.4],
        [22.7, 37.7, 47.4, 50.7, 48.1, 44.0, 40.6, 38.5, 38.1, 36.5, 34.7, 34.1, 29.7, 23.3, 17.8, 15.2, 16.6, 17.5, 20.1, 16.7, 11.1, 10.8, 6.1, 2.8, 2.5, 3.2, -0.2, 5.1, 2.5, -4.1, -5.3, -6.2, -10.7, -15.9, -19.1, -16.7, -23.4, -30.3, -27.0, -28.2, -33.9, -33.8, -33.5, -36.5, -37.8, -40.5, -43.3, -45.9, -47.6, -48.2, -48.2],
    ],
];

/// Which measured body.
pub const VIOLIN: u8 = 0;
pub const VIOLA: u8 = 1;
pub const CELLO: u8 = 2;
pub const BASS: u8 = 3;

/// The velocities the three dynamics were measured at (pp, mf, ff).
const AT: [f32; 3] = [0.26, 0.62, 0.87];

fn knot(curve: &[f32; KNOTS], hz: f32) -> f32 {
    let x = ((hz.max(1.0) / KNOT0).log2() * PER_OCTAVE).clamp(0.0, (KNOTS - 1) as f32);
    let i = (x as usize).min(KNOTS - 2);
    let t = x - i as f32;
    curve[i] * (1.0 - t) + curve[i + 1] * t
}

/// The level (dB, 0 at 1 kHz) of a harmonic at `hz` on a body, at
/// dynamic `d` (0..1): between the measured dynamics, a little beyond them.
pub fn level(body: u8, hz: f32, d: f32) -> f32 {
    let c = &BODY[body as usize];
    let (a, b, t) = if d < AT[1] { (0, 1, (d - AT[0]) / (AT[1] - AT[0])) } else { (1, 2, (d - AT[1]) / (AT[2] - AT[1])) };
    let t = t.clamp(-0.5, 1.5);
    let (la, lb) = (knot(&c[a], hz), knot(&c[b], hz));
    la + (lb - la) * t
}

/// A section's body: many instruments, their resonances averaged (a third
/// of an octave).
pub fn section_level(body: u8, hz: f32, d: f32) -> f32 {
    let k = 2f32.powf(1.0 / 6.0);
    (level(body, hz / k, d) + level(body, hz, d) + level(body, hz * k, d)) / 3.0
}

/// One instrument's own resonances over the measured average: fixed peaks
/// and dips of a few dB between 250 Hz and 6 kHz, the same every time (an
/// instrument does not change), different for each body.
pub fn ripple(body: u8, hz: f32) -> f32 {
    if !(200.0..7000.0).contains(&hz) {
        return 0.0;
    }
    let o = hz.log2();
    let s = body as f32 * 1.7;
    let w = ((o - 7.64) / 0.5).clamp(0.0, 1.0) * ((12.77 - o) / 0.5).clamp(0.0, 1.0);
    w * (2.2 * (std::f32::consts::TAU * o / 0.17 + s).sin() + 1.4 * (std::f32::consts::TAU * o / 0.29 + 2.3 * s).sin() + 0.8 * (std::f32::consts::TAU * o / 0.11 + 0.7 * s).sin())
}

/// A curve at fine resolution, as linear gains: a forty-eighth of an
/// octave from [`KNOT0`] up, for quick lookups by harmonic.
pub const FINE_PER_OCTAVE: f32 = 48.0;
pub const FINE: usize = 410;

/// The gains of `body` at dynamic `d`, a section's (averaged) or one
/// instrument's (with its ripple); `shape(hz)`: extra dB (a technique).
pub fn fine_curve(body: u8, d: f32, section: bool, shape: impl Fn(f32) -> f32, out: &mut [f32; FINE]) {
    for (i, g) in out.iter_mut().enumerate() {
        let hz = KNOT0 * 2f32.powf(i as f32 / FINE_PER_OCTAVE);
        let l = if section { section_level(body, hz, d) } else { level(body, hz, d) + ripple(body, hz) };
        *g = 10f32.powf((l + shape(hz)) / 20.0);
    }
}

/// The gain at `hz` from a fine curve.
#[inline]
pub fn at(curve: &[f32; FINE], log2_hz: f32) -> f32 {
    let x = ((log2_hz - KNOT0.log2()) * FINE_PER_OCTAVE).clamp(0.0, (FINE - 1) as f32);
    let i = (x as usize).min(FINE - 2);
    let t = x - i as f32;
    curve[i] + (curve[i + 1] - curve[i]) * t
}
