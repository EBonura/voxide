//! Piece planning, the mix and the resampler.

use loop_mixer::{mix, plan, resample::resample, rng::Rng, wav};

const LENGTHS: [f64; 6] = [57.313, 51.2, 53.333, 53.333, 52.603, 56.471];

#[test]
fn a_set_uses_every_loop_never_repeats_a_loop_in_a_piece_and_differs() {
    let pieces = plan::set(20_261_015, &LENGTHS, 4, 168.0);
    assert_eq!(pieces.len(), 4);
    for p in &pieces {
        assert!(plan::seconds(p, &LENGTHS) <= 168.0);
        assert!(p.len() >= 2);
        for (i, s) in p.iter().enumerate() {
            assert!(p[..i].iter().all(|o| o.loop_index != s.loop_index), "a loop twice in one piece");
        }
        assert!(p.iter().all(|s| (1..=3).contains(&s.repeats)));
    }
    for l in 0..6 {
        assert!(pieces.iter().flatten().any(|s| s.loop_index == l), "loop {l} unused");
    }
    for (i, p) in pieces.iter().enumerate() {
        assert!(!pieces[..i].contains(p));
    }
}

#[test]
fn a_seed_names_the_set() {
    let a = plan::set(7, &LENGTHS, 4, 168.0);
    assert_eq!(a, plan::set(7, &LENGTHS, 4, 168.0));
    assert_ne!(a, plan::set(8, &LENGTHS, 4, 168.0));
}

#[test]
fn resampling_keeps_a_tone_and_its_level() {
    let n = 44_100;
    let tone: Vec<f64> = (0..n).map(|i| 0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 44_100.0).sin()).collect();
    let out = resample(&tone, 44_100, 37_800);
    assert_eq!(out.len(), 37_800);
    for (i, v) in out.iter().enumerate().skip(200).take(37_000) {
        let want = 0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 37_800.0).sin();
        assert!((v - want).abs() < 1e-3, "{i}: {v} vs {want}");
    }
}

#[test]
fn resampling_removes_what_the_new_rate_cannot_hold() {
    let n = 44_100;
    let tone: Vec<f64> = (0..n).map(|i| (2.0 * std::f64::consts::PI * 20_000.0 * i as f64 / 44_100.0).sin()).collect();
    let out = resample(&tone, 44_100, 37_800);
    let peak = out[200..out.len() - 200].iter().fold(0.0f64, |m, v| m.max(v.abs()));
    assert!(peak < 0.003, "20 kHz leaked through at {peak}");
}

fn loop_of(freq: f64, secs: usize) -> wav::Audio {
    let n = 44_100 * secs;
    let ch: Vec<f64> = (0..n).map(|i| 0.4 * (2.0 * std::f64::consts::PI * freq * i as f64 / 44_100.0).sin()).collect();
    wav::Audio { rate: 44_100, ch: [ch.clone(), ch] }
}

#[test]
fn crossfades_are_equal_power_and_the_ends_fade() {
    // Two loops that are whole-cycle tones, joined once.
    let loops = [loop_of(200.0, 10), loop_of(300.0, 10)];
    let piece = vec![
        plan::Segment { loop_index: 0, repeats: 1 },
        plan::Segment { loop_index: 1, repeats: 1 },
    ];
    let out = mix::mix(&piece, &loops);
    assert_eq!(out[0].len(), 20 * 44_100);
    // the start is silent and rises, the end falls to silence
    assert!(out[0][0].abs() < 1e-9);
    assert!(out[0][out[0].len() - 1].abs() < 1e-3);
    // mid-fade (the boundary) both are at -3 dB: sin(pi/4) = cos(pi/4)
    let mid = 10 * 44_100;
    let a = loops[0].ch[0][(mid as i64 - 10 * 44_100).rem_euclid(441_000) as usize];
    let b = loops[1].ch[0][0];
    let want = (a + b) * std::f64::consts::FRAC_1_SQRT_2;
    assert!((out[0][mid] - want).abs() < 1e-9);
    // far from any fade the piece is just the loop
    assert!((out[0][3 * 44_100 + 17] - loops[0].ch[0][3 * 44_100 + 17]).abs() < 1e-12);
    assert!((out[0][12 * 44_100 + 5] - loops[1].ch[0][2 * 44_100 + 5]).abs() < 1e-12);
}

#[test]
fn render_dithers_to_16_bits_and_limits_the_peak() {
    let loops = [loop_of(200.0, 5)];
    let piece = vec![plan::Segment { loop_index: 0, repeats: 1 }];
    let hot = wav::Audio { rate: 44_100, ch: [loops[0].ch[0].iter().map(|v| v * 3.0).collect(), loops[0].ch[1].iter().map(|v| v * 3.0).collect()] };
    let r = mix::render(&piece, &[hot], &mut Rng::new(1));
    assert!(r.gain < 1.0);
    let peak = r.left.iter().map(|v| v.unsigned_abs()).max().unwrap();
    assert!(peak as f64 <= 0.97 * 32768.0 + 2.0);
    assert_eq!(r.left.len(), 5 * 37_800);
}

#[test]
fn wav_round_trips_16_bit() {
    let bytes = wav::write16(37_800, &[1, -2, 3], &[4, 5, -6]);
    let a = wav::read(&bytes).unwrap();
    assert_eq!(a.rate, 37_800);
    assert_eq!(a.ch[0].len(), 3);
    assert!((a.ch[1][2] - -6.0 / 32768.0).abs() < 1e-12);
}
