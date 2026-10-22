//! The settings record on real cards. The game no longer reads or writes the
//! controls-seen marks (the game flag bit and the old difficulty-2 value), so
//! every older card must load and save back byte for byte as it is.

use psx_mc::{Card, RamCard};
use psx_pad::{ActionBinding, ActionMap};
use psx_settings::{Profile, FLAG_INVERT_Y, MAX_RECORD_LEN};

const SETTINGS_FILE: &str = "BESLES-00000VOXSET1";
/// Difficulty 2 and no game flag: the prior build's "controls seen" card.
const LEGACY_IMAGE: &[u8] = include_bytes!("../../../tests/cards/legacy-controls-seen.mcd");
/// Game flag bit 1 and difficulty 1: the card the 0.4.0 migration wrote.
const FLAGGED_IMAGE: &[u8] = include_bytes!("../../../tests/cards/flagged-controls-seen.mcd");

fn profile() -> Profile<5, 0> {
    Profile::new(ActionMap::new([ActionBinding::UNBOUND; 5]))
}

fn round_trip(profile: &Profile<5, 0>) -> Profile<5, 0> {
    let mut bytes = [0u8; MAX_RECORD_LEN];
    let len = profile.encode(&mut bytes).unwrap();
    assert_eq!(len, 38);
    assert_eq!(bytes[4], 2, "the record version does not change");
    Profile::decode(&bytes[..len]).unwrap()
}

fn card_profile(image: &[u8]) -> Profile<5, 0> {
    let mut card = Card::new(RamCard::from_image(image).unwrap());
    card.validate_filesystem().unwrap();
    let mut bytes = [0u8; MAX_RECORD_LEN];
    let len = card.read(SETTINGS_FILE, &mut bytes).unwrap();
    assert_eq!(len, 38);
    Profile::decode(&bytes[..len]).unwrap()
}

#[test]
fn legacy_card_keeps_its_marker_through_a_save() {
    let old = card_profile(LEGACY_IMAGE);
    assert_eq!(old.difficulty, 2);
    assert_eq!(old.game_flags(), 0);
    assert_eq!(round_trip(&old), old);
}

#[test]
fn flagged_card_keeps_its_flag_through_a_save() {
    let current = card_profile(FLAGGED_IMAGE);
    assert_eq!(current.difficulty, 1);
    assert_eq!(current.game_flags() & (1 << 1), 2);
    assert_eq!(round_trip(&current), current);
}

#[test]
fn a_fresh_profile_and_the_other_flags_are_untouched() {
    let fresh = profile();
    assert_eq!(fresh.difficulty, 1);
    assert_eq!(fresh.game_flags(), 0);
    assert_eq!(round_trip(&fresh), fresh);

    let mut other = profile();
    other.flags = FLAG_INVERT_Y | 0xa0;
    other.look_speed_percent = 140;
    assert_eq!(round_trip(&other), other);
}
