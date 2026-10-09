//! Compile the current near-grid expressions against an independent
//! span/division oracle.
//!
//! The game is no_std/MIPS; extracting the exact arithmetic lets the host test
//! all packed face dimensions without mocking the GPU or maintaining a second
//! copy of the candidate expressions. The face corners come from `face_verts`,
//! the grid steps from the axis `match` in `emit_near_face`, and the camera
//! planes from its `plane_du`/`plane_dv` closures.

mod common;

/// The oracle: the old divide-the-span construction, checked against the
/// extracted axis selection and camera planes for every direction, packed
/// dimension and signed camera row.
const REFERENCE: &str = r#"
#[test]
fn exact_steps_for_all_face_directions_and_packed_dimensions() {
    let mut cases = 0;
    for dir in 0..6 {
        for w in 1..=16 {
            for h in 1..=8 {
                let v = face(3, 20, 4, dir, w, h);
                let (uc, vc) = if dir < 2 { (h, w) } else { (w, h) };
                let old_du = (
                    (v[1].0 - v[0].0) / uc as i32,
                    (v[1].1 - v[0].1) / uc as i32,
                    (v[1].2 - v[0].2) / uc as i32,
                );
                let old_dv = (
                    (v[2].0 - v[0].0) / vc as i32,
                    (v[2].1 - v[0].1) / vc as i32,
                    (v[2].2 - v[0].2) / vc as i32,
                );
                let (du, dv) = steps(dir);
                assert_eq!((du, dv), (old_du, old_dv));
                for row in [
                    [4096, 0, 0],
                    [0, 4096, 0],
                    [0, 0, 4096],
                    [-4096, 0, 0],
                    [0, -4096, 0],
                    [0, 0, -4096],
                    [1771, -2315, 3177],
                    [-1789, 2943, -2559],
                ] {
                    let dot = |p: (i32, i32, i32)| row[0] * p.0 + row[1] * p.1 + row[2] * p.2;
                    let (a, b) = planes(dir, row);
                    assert_eq!((a, b), (dot(old_du), dot(old_dv)));
                    for u in 0..=uc {
                        for v0 in 0..=vc {
                            let p = (
                                v[0].0 + du.0 * u as i32 + dv.0 * v0 as i32,
                                v[0].1 + du.1 * u as i32 + dv.1 * v0 as i32,
                                v[0].2 + du.2 * u as i32 + dv.2 * v0 as i32,
                            );
                            assert_eq!(
                                (dot(v[0]) + a * u as i32 + b * v0 as i32) >> 12,
                                dot(p) >> 12
                            );
                            cases += 1;
                        }
                    }
                }
            }
        }
    }
    println!("{} exact grid/row comparisons", cases);
}
"#;

/// The text of the item starting at `signature`, through its matching brace.
fn item<'a>(text: &'a str, signature: &str) -> &'a str {
    let start = text
        .find(signature)
        .unwrap_or_else(|| panic!("{signature:?} not found"));
    let open = start + text[start..].find('{').expect("opening brace");
    let mut depth = 1;
    for (offset, c) in text[open + 1..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return &text[start..open + 1 + offset + 1];
        }
    }
    panic!("{signature:?} has no closing brace");
}

/// The text from `start` (searched after `anchor`) up to, not including, `end`.
fn between<'a>(text: &'a str, anchor: &str, start: &str, end: &str) -> &'a str {
    let anchor_at = text
        .find(anchor)
        .unwrap_or_else(|| panic!("{anchor:?} not found"));
    let first = anchor_at
        + text[anchor_at..]
            .find(start)
            .unwrap_or_else(|| panic!("{start:?} not found after {anchor:?}"));
    let last = first
        + text[first..]
            .find(end)
            .unwrap_or_else(|| panic!("{end:?} not found after {start:?}"));
    &text[first..last]
}

#[test]
fn all_packed_grid_steps_and_camera_planes() {
    let root = common::root();
    let source = std::fs::read_to_string(root.join("game/src/main.rs")).expect("game/src/main.rs");
    let block = {
        let at = source.find("const BLOCK: i32 = ").expect("BLOCK") + "const BLOCK: i32 = ".len();
        let digits: String = source[at..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits
    };
    let vertices = item(&source, "fn face_verts(");
    let steps = between(
        &source,
        "fn emit_near_face(",
        "let (du, dv) = match dir",
        "    // Exact q12 camera-space",
    );
    let planes = between(
        &source,
        "fn emit_near_face(",
        "let plane_du =",
        "let camera_base =",
    );
    let mut program = format!("const BLOCK:i32={block};\n");
    program += &vertices.replacen("fn face_verts(", "fn face(", 1);
    program += "\nfn steps(dir:usize)->((i32,i32,i32),(i32,i32,i32)){\n";
    program += steps;
    program += "(du,dv)\n}\n";
    program += "fn planes(dir:usize,row:[i32;3])->(i32,i32){\n";
    program += planes;
    program += "(plane_du(row),plane_dv(row))\n}\n";
    program += REFERENCE;

    let scratch = common::Scratch::new("vox-grid-proof-");
    let output = common::compile_and_run(
        &scratch,
        &program,
        &["--edition=2021", "--test"],
        &["--nocapture"],
        &root,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("321024 exact grid/row comparisons"),
        "unexpected output:\n{stdout}"
    );
}
