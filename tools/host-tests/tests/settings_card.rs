#[path = "../../../game/src/settings_menu.rs"]
mod settings_menu;

use psx_mc::{Card, RamCard};
use psx_pad::{ActionBinding, ActionMap};
use psx_settings::Profile;
use settings_menu::{save_to_card, CardSave};

const FILE: &str = "BESLES-00000VOXSET1";
const TITLE: &str = "VoXide Settings";

fn profile() -> Profile<5, 0> {
    Profile::new(ActionMap::new([ActionBinding::UNBOUND; 5]))
}

#[test]
fn first_press_on_a_blank_card_asks_and_leaves_it_alone() {
    let mut card = Card::new(RamCard::new());
    assert_eq!(
        save_to_card(&mut card, FILE, TITLE, &profile(), false),
        CardSave::BlankCard
    );
    assert!(card.into_inner().image().iter().all(|b| *b == 0));
}

#[test]
fn second_press_formats_the_blank_card_and_saves() {
    let mut card = Card::new(RamCard::new());
    assert_eq!(
        save_to_card(&mut card, FILE, TITLE, &profile(), false),
        CardSave::BlankCard
    );
    assert_eq!(
        save_to_card(&mut card, FILE, TITLE, &profile(), true),
        CardSave::Saved
    );
    card.validate_filesystem().unwrap();
    let mut bytes = [0u8; psx_settings::MAX_RECORD_LEN];
    let len = card.read(FILE, &mut bytes).unwrap();
    assert_eq!(Profile::<5, 0>::decode(&bytes[..len]).unwrap(), profile());
}

#[test]
fn a_formatted_card_saves_at_once_and_keeps_its_other_files() {
    let mut card = Card::new(RamCard::new());
    card.format().unwrap();
    card.write("BESLES-00000OTHER01", "OTHER", b"keep me").unwrap();
    assert_eq!(
        save_to_card(&mut card, FILE, TITLE, &profile(), false),
        CardSave::Saved
    );
    let mut bytes = [0u8; 64];
    let len = card.read("BESLES-00000OTHER01", &mut bytes).unwrap();
    assert_eq!(&bytes[..len], b"keep me");
}
