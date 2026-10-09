//! The converters against the repository's real assets.

use voxide_tools::{pack, png, repo_root, sfx};

#[test]
fn sfx_bank_matches_the_committed_chunk() {
    let root = repo_root();
    let (bank, rs) = sfx::cook(&root, false).expect("cook the sound effects");
    let committed =
        std::fs::read(root.join("assets/sfx/pak/chunk_3000.bin")).expect("chunk_3000.bin");
    assert!(
        bank == committed,
        "cooked bank differs from assets/sfx/pak/chunk_3000.bin"
    );
    assert!(rs.contains("pub static SAMPLES: [Sample; 25]"));
    assert!(rs.contains("Sample { off: 0, rate: 22050, blocks: 234 }, // step_grass"));
}

#[test]
fn dirt_tile_palette_and_indices() {
    let root = repo_root();
    let file = std::fs::read(root.join("assets/pack/blocks/blocks/dirt.png")).expect("dirt.png");
    let image = png::decode(&file).expect("decode dirt.png");
    assert_eq!((image.width, image.height), (16, 16));
    let (palette, indices) = pack::convert_tile(&image, None, None).expect("convert dirt");
    // Pillow's median cut order for this tile (five colours, padded with black).
    let expected: [[u8; 3]; 5] = [
        [139, 99, 93],
        [130, 86, 70],
        [120, 80, 65],
        [109, 78, 75],
        [120, 70, 61],
    ];
    assert_eq!(&palette[..5], &expected);
    assert!(palette[5..].iter().all(|c| *c == [0, 0, 0]));
    for (pixel, &index) in image.rgba.iter().zip(indices.iter()) {
        assert_eq!(palette[usize::from(index)], [pixel[0], pixel[1], pixel[2]]);
    }
}

#[test]
fn recolors_keep_the_padding_entries_in_play() {
    // Leaves remap every one of the 16 entries, including the black padding.
    let root = repo_root();
    let file = std::fs::read(root.join("assets/pack/blocks/blocks/oak_leaves.png")).expect("png");
    let image = png::decode(&file).expect("decode");
    let (plain, _) = pack::convert_tile(&image, Some([32, 56, 26]), None).expect("plain");
    let (leaves, _) = pack::convert_tile(&image, Some([32, 56, 26]), Some(pack::Recolor::Leaves))
        .expect("leaves");
    assert_eq!(plain[15], [0, 0, 0]);
    assert_eq!(leaves[15], [28, 42, 17]);
}
