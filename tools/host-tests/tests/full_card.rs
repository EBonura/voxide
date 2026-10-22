//! tests/fixtures/full.mcd: a formatted memory card with every block taken by
//! other games' files, for the SAVE AND QUIT journey that must fail with CARD
//! FULL. The test rebuilds it and compares; `WRITE_FIXTURE=1 cargo test
//! --test full_card` rewrites the file.

use psx_mc::{Card, RamCard};

fn build() -> Vec<u8> {
    let mut card = Card::new(RamCard::new());
    card.format().unwrap();
    let payload = [0x5au8; 100];
    for i in 0..psx_mc::DATA_BLOCKS {
        let name = format!("BESLES-00000FULL{i:02}");
        card.write(&name, "Other game", &payload).unwrap();
    }
    card.validate_filesystem().unwrap();
    // No room left for even a one-block file.
    assert!(card
        .write("BESLES-00000VOXEL1X", "VoXide", &payload)
        .is_err());
    card.into_inner().image().to_vec()
}

#[test]
fn the_full_card_fixture_is_the_generated_one() {
    let image = build();
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/full.mcd");
    if std::env::var("WRITE_FIXTURE").is_ok() {
        std::fs::write(path, &image).unwrap();
    }
    assert_eq!(std::fs::read(path).unwrap(), image);
}
