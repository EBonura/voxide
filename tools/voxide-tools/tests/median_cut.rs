//! The pack's palettes come from Pillow's median cut. These cases were
//! recorded from it (see the header of `median_cut_cases.txt`); the port must
//! reproduce the palette order exactly, and map every pixel to its entry.

use voxide_tools::pack::quantize;

fn hex(h: &str) -> [u8; 3] {
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).expect("hex colour");
    [byte(0), byte(2), byte(4)]
}

#[test]
fn matches_recorded_pillow_palettes() {
    let text = include_str!("median_cut_cases.txt");
    let mut cases = 0;
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let (input, expected) = line.split_once(" | ").expect("case separator");
        let mut pixels = Vec::new();
        for item in input.split(' ') {
            let (color, count) = item.split_once(':').expect("colour:count");
            let count: usize = count.parse().expect("count");
            pixels.extend(std::iter::repeat_n(hex(color), count));
        }
        assert_eq!(pixels.len(), 256);
        let expected: Vec<[u8; 3]> = expected.split(' ').map(hex).collect();
        let (palette, indices) = quantize(&pixels).expect("at most 16 colours");
        assert_eq!(palette, expected, "case {cases}");
        for (pixel, &index) in pixels.iter().zip(&indices) {
            assert_eq!(&palette[usize::from(index)], pixel);
        }
        cases += 1;
    }
    assert_eq!(cases, 150);
}

#[test]
fn more_than_sixteen_colours_is_refused() {
    let pixels: Vec<[u8; 3]> = (0..17u8).map(|i| [i * 10, 0, 0]).collect();
    assert!(quantize(&pixels).is_err());
}
