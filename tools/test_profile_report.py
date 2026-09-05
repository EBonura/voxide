import contextlib
import io
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import profile_report


class ProfileReportTests(unittest.TestCase):
    def run_report(self, data):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "profile.csv"
            path.write_text(data)
            output = io.StringIO()
            with patch("sys.argv", ["profile_report.py", str(path)]), contextlib.redirect_stdout(output):
                result = profile_report.main()
            return result, output.getvalue()

    def test_terminal_marker_is_not_a_free_frame_and_generation_uses_stage_25(self):
        result, output = self.run_report(
            "frame_cycles,cell_collect,cd_room_chunk_load,cd_world_pack_stream\n"
            "1142472,700000,900000,100000\n0,0,0,0\n"
        )
        self.assertEqual(result, 0)
        self.assertIn("completed frames: 1", output)
        self.assertIn("30.0 fps", output)
        generation = next(line for line in output.splitlines() if "generation" in line)
        self.assertIn("100,000", generation)
        self.assertNotIn("900,000", generation)

    def test_unexported_generation_and_no_completed_frames(self):
        result, output = self.run_report("frame_cycles,cd_room_chunk_load\n1142472,900000\n")
        self.assertEqual(result, 0)
        self.assertNotIn("generation", output)
        result, _ = self.run_report("frame_cycles\n0\n")
        self.assertEqual(result, 1)


if __name__ == "__main__":
    unittest.main()
