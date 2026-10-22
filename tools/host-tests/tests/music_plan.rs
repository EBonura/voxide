//! Java's music timing and the pick, against the pieces in musicdata.rs.

#[path = "../../../game/src/music_plan.rs"]
mod music_plan;
#[path = "../../../game/src/musicdata.rs"]
mod musicdata;

use music_plan::*;
use musicdata::*;

/// Sim ticks (vblanks) in a second.
const SECOND: u32 = 60;

#[test]
fn menu_delay_is_one_to_thirty_seconds() {
    let all: Vec<u32> = (0..2000).map(|r| delay_vblanks(MENU_DELAY, r)).collect();
    assert_eq!(*all.iter().min().unwrap(), SECOND);
    assert_eq!(*all.iter().max().unwrap(), 30 * SECOND);
    // 20 and 600 game ticks, three vblanks each
    assert_eq!(delay_vblanks(MENU_DELAY, 0), 20 * 3);
    assert_eq!(delay_vblanks(MENU_DELAY, 580), 600 * 3);
}

#[test]
fn game_delay_is_ten_to_twenty_minutes() {
    assert_eq!(delay_vblanks(GAME_DELAY, 0), 10 * 60 * SECOND);
    assert_eq!(delay_vblanks(GAME_DELAY, 12_000), 20 * 60 * SECOND);
    assert_eq!(delay_vblanks(GAME_DELAY, 12_001), 10 * 60 * SECOND);
    for r in (0..1_000_000u32).step_by(997) {
        let d = delay_vblanks(GAME_DELAY, r);
        assert!((36_000..=72_000).contains(&d), "{d}");
    }
    assert_eq!(START_DELAY * 3, 5 * SECOND);
}

#[test]
fn every_piece_can_be_picked_and_a_pick_stays_in_the_table() {
    let seen: std::collections::BTreeSet<usize> = (0..200u32).map(pick).collect();
    assert_eq!(seen.len(), TRACKS.len());
    assert!(seen.iter().all(|t| *t < TRACKS.len()));
}

#[test]
fn pieces_are_a_few_minutes_each_and_fit_the_interleave() {
    for t in TRACKS.iter() {
        assert!(t.file >= 1 && t.file as usize <= FILES);
        assert!(t.channel < 4);
        assert!(
            (120_000..=200_000).contains(&t.length_ms),
            "{} {} ms",
            t.name,
            t.length_ms
        );
    }
}

#[test]
fn slider_maps_to_the_drive_mixer_six_db_under_the_cooked_level() {
    assert_eq!(mixer_level(0), 0);
    assert_eq!(mixer_level(100), 0x40);
    assert_eq!(mixer_level(50), 0x20);
    assert_eq!(mixer_level(25), 16);
    assert_eq!(mixer_level(200), 0x40);
}
