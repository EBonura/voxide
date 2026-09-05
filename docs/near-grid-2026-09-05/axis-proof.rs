const BLOCK: i32 = 64;
fn face(lx: i32, by: i32, lz: i32, dir: usize, w: usize, h: usize) -> [(i32, i32, i32); 4] {
    let (ex, ey, ez) = match dir {
        0 | 1 => (1, w as i32, h as i32),
        2 | 3 => (w as i32, 1, h as i32),
        _ => (w as i32, h as i32, 1),
    };
    // CHUNK-LOCAL corner coordinates (gte_begin_chunk pointed the GTE's TR at
    // this chunk's origin): all values fit i16 raw, no per-corner subtraction.
    let x0 = lx * BLOCK;
    let x1 = x0 + ex * BLOCK;
    let y0 = by * BLOCK;
    let y1 = y0 + ey * BLOCK;
    let z0 = lz * BLOCK;
    let z1 = z0 + ez * BLOCK;

    let verts = match dir {
        0 => [(x1, y1, z0), (x1, y1, z1), (x1, y0, z0), (x1, y0, z1)],
        1 => [(x0, y1, z1), (x0, y1, z0), (x0, y0, z1), (x0, y0, z0)],
        2 => [(x0, y1, z0), (x1, y1, z0), (x0, y1, z1), (x1, y1, z1)],
        3 => [(x0, y0, z1), (x1, y0, z1), (x0, y0, z0), (x1, y0, z0)],
        4 => [(x1, y1, z1), (x0, y1, z1), (x1, y0, z1), (x0, y0, z1)],
        _ => [(x0, y1, z0), (x1, y1, z0), (x0, y0, z0), (x1, y0, z0)],
    };

    verts
}
fn steps(dir: usize) -> ((i32, i32, i32), (i32, i32, i32)) {
    let (du, dv) = match dir {
        0 => ((0, 0, BLOCK), (0, -BLOCK, 0)),
        1 => ((0, 0, -BLOCK), (0, -BLOCK, 0)),
        2 => ((BLOCK, 0, 0), (0, 0, BLOCK)),
        3 => ((BLOCK, 0, 0), (0, 0, -BLOCK)),
        4 => ((-BLOCK, 0, 0), (0, -BLOCK, 0)),
        _ => ((BLOCK, 0, 0), (0, -BLOCK, 0)),
    };
    (du, dv)
}
fn planes(dir: usize, row: [i32; 3]) -> (i32, i32) {
    let plane_du = |row: [i32; 3]| -> i32 {
        match dir {
            0 => row[2] * BLOCK,
            1 => -row[2] * BLOCK,
            4 => -row[0] * BLOCK,
            _ => row[0] * BLOCK,
        }
    };
    let plane_dv = |row: [i32; 3]| -> i32 {
        match dir {
            2 => row[2] * BLOCK,
            3 => -row[2] * BLOCK,
            _ => -row[1] * BLOCK,
        }
    };
    (plane_du(row), plane_dv(row))
}
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
