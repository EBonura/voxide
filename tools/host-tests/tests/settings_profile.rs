#[path = "../../../game/src/settings_profile.rs"]
mod settings_profile;

use psx_mc::{Card, Entry, RamCard, MAX_NAME_LEN};
use psx_pad::{ActionBinding, ActionMap};
use psx_settings::{Profile, FLAG_INVERT_Y, MAX_RECORD_LEN};

fn profile() -> Profile<5, 0> {
    Profile::new(ActionMap::new([ActionBinding::UNBOUND; 5]))
}

fn round_trip(profile: &Profile<5, 0>) -> Profile<5, 0> {
    let mut bytes = [0u8; MAX_RECORD_LEN];
    let len = profile.encode(&mut bytes).unwrap();
    assert_eq!(len, 38);
    assert_eq!(bytes[4], 2);
    Profile::decode(&bytes[..len]).unwrap()
}

#[test]
fn fresh_card_still_shows_controls_once() {
    let mut fresh = profile();
    assert!(!settings_profile::migrate_legacy_controls_seen(&mut fresh));
    assert_eq!(fresh.difficulty, 1);
    assert_eq!(fresh.game_flags() & settings_profile::CONTROLS_SEEN, 0);

    fresh.set_game_flag(settings_profile::CONTROLS_SEEN, true);
    let loaded = round_trip(&fresh);
    assert_eq!(loaded.difficulty, 1);
    assert_ne!(loaded.game_flags() & settings_profile::CONTROLS_SEEN, 0);
}

#[test]
fn exact_legacy_seen_value_moves_to_the_game_flag() {
    let mut legacy = profile();
    legacy.difficulty = 2;
    legacy.flags = FLAG_INVERT_Y | 0xA0;
    legacy.look_speed_percent = 140;
    let mut loaded = round_trip(&legacy);

    assert!(settings_profile::migrate_legacy_controls_seen(&mut loaded));
    assert_eq!(loaded.difficulty, 1);
    assert_eq!(
        loaded.flags,
        FLAG_INVERT_Y | 0xA0 | settings_profile::CONTROLS_SEEN
    );
    assert_eq!(loaded.look_speed_percent, 140);
    assert_eq!(round_trip(&loaded), loaded);
}

#[test]
fn flagged_card_and_other_difficulty_values_are_unchanged() {
    let mut flagged = profile();
    flagged.flags = 0xA0 | settings_profile::CONTROLS_SEEN;
    assert!(!settings_profile::migrate_legacy_controls_seen(
        &mut flagged
    ));
    assert_eq!(round_trip(&flagged), flagged);

    let mut other = profile();
    other.difficulty = 3;
    other.flags = FLAG_INVERT_Y | 0xA0;
    assert!(!settings_profile::migrate_legacy_controls_seen(&mut other));
    assert_eq!(other.difficulty, 3);
    assert_eq!(other.flags, FLAG_INVERT_Y | 0xA0);
}

const SETTINGS_FILE: &str = "BESLES-00000VOXSET1";
const LEGACY_IMAGE: &[u8] = include_bytes!("../../../tests/cards/legacy-controls-seen.mcd");
const MIGRATED_IMAGE: &[u8] = include_bytes!("../../../tests/cards/flagged-controls-seen.mcd");

fn card_profile(image: &[u8]) -> Profile<5, 0> {
    let mut card = Card::new(RamCard::from_image(image).unwrap());
    card.validate_filesystem().unwrap();
    let mut bytes = [0u8; MAX_RECORD_LEN];
    let len = card.read(SETTINGS_FILE, &mut bytes).unwrap();
    assert_eq!(len, 38);
    Profile::decode(&bytes[..len]).unwrap()
}

#[test]
fn real_card_migration_preserves_all_other_settings() {
    let old = card_profile(LEGACY_IMAGE);
    let current = card_profile(MIGRATED_IMAGE);
    assert_eq!(old.difficulty, 2);
    assert_eq!(old.game_flags() & settings_profile::CONTROLS_SEEN, 0);
    assert_eq!(current.difficulty, 1);
    assert_eq!(current.game_flags() & settings_profile::CONTROLS_SEEN, 2);
    let mut expected = old;
    expected.difficulty = 1;
    expected.set_game_flag(settings_profile::CONTROLS_SEEN, true);
    assert_eq!(current, expected);

    fn files(image: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut card = Card::new(RamCard::from_image(image).unwrap());
        let blank = Entry {
            name: [0; MAX_NAME_LEN + 1],
            name_len: 0,
            blocks: 0,
        };
        let mut entries = [blank; 15];
        let count = card.list(&mut entries).unwrap();
        let mut result = Vec::new();
        for entry in &entries[..count] {
            let mut payload = vec![0u8; 128 * 1024];
            let len = card.read(entry.name(), &mut payload).unwrap();
            payload.truncate(len);
            result.push((entry.name().to_string(), payload));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }
    let old_files = files(LEGACY_IMAGE);
    let new_files = files(MIGRATED_IMAGE);
    assert_eq!(
        old_files.len(),
        1,
        "the seen-card fixture contains settings only"
    );
    assert_eq!(old_files.len(), new_files.len());
    for ((old_name, old_payload), (new_name, new_payload)) in old_files.iter().zip(&new_files) {
        assert_eq!(old_name, new_name);
        if old_name != SETTINGS_FILE {
            assert_eq!(old_payload, new_payload, "unrelated save file {old_name}");
        }
    }
}
