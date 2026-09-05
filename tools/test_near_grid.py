#!/usr/bin/env python3
"""Compile current near-grid expressions against an independent span/division oracle.

The game is no_std/MIPS; extracting the exact arithmetic lets the host test all
packed face dimensions without mocking the GPU or maintaining a second copy of
the candidate expressions. Requires rustc on PATH.
"""
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

REFERENCE = r"""
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
"""


def section(source, function, start, end):
    first = source.index(start, source.index(function))
    return source[first:source.index(end, first)]


class NearGridTests(unittest.TestCase):
    def test_all_packed_grid_steps_and_camera_planes(self):
        source = (ROOT / "game/src/main.rs").read_text()
        block = re.search(r"const BLOCK: i32 = (\d+);", source).group(1)
        vertices = section(source, "fn emit_face(", "    let (ex, ey, ez)", "    // Kick RTPT")
        steps = section(source, "fn emit_near_face(", "    let (du, dv) = match dir", "    // Exact q12 camera-space")
        planes = section(source, "fn emit_near_face(", "    let plane_du =", "    let camera_base =")
        program = f"const BLOCK:i32={block};\n"
        program += "fn face(lx:i32,by:i32,lz:i32,dir:usize,w:usize,h:usize)->[(i32,i32,i32);4]{\n" + vertices + "verts\n}\n"
        program += "fn steps(dir:usize)->((i32,i32,i32),(i32,i32,i32)){\n" + steps + "(du,dv)\n}\n"
        program += "fn planes(dir:usize,row:[i32;3])->(i32,i32){\n" + planes + "(plane_du(row),plane_dv(row))\n}\n"
        program += REFERENCE
        with tempfile.TemporaryDirectory(prefix="vox-grid-proof-") as directory:
            root = Path(directory)
            (root / "proof.rs").write_text(program)
            subprocess.run(["rustc", "--edition=2021", "--test", str(root / "proof.rs"), "-o", str(root / "proof")], check=True)
            result = subprocess.run([str(root / "proof"), "--nocapture"], text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertIn("321024 exact grid/row comparisons", result.stdout)


if __name__ == "__main__":
    unittest.main()
