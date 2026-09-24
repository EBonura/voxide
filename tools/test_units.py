#!/usr/bin/env python3
"""Host test for game/src/units.rs: compile its #[cfg(test)] module with the
repository's pinned toolchain (rustc --test, host target) and run it. Pins the
real-unit conversions every tuning constant goes through, e.g. 1 s == 60 sim
ticks and 20 Java ticks == 60 sim ticks."""
import os, subprocess, sys, tempfile, unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


class UnitsTests(unittest.TestCase):
    def test_units_rs(self):
        src = os.path.join(ROOT, "game", "src", "units.rs")
        with tempfile.TemporaryDirectory() as d:
            exe = os.path.join(d, "units_test")
            subprocess.run(["rustc", "--edition", "2021", "--test", src, "-o", exe],
                           cwd=ROOT, check=True, capture_output=True, text=True)
            r = subprocess.run([exe], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertIn("test result: ok", r.stdout)


if __name__ == "__main__":
    unittest.main()
