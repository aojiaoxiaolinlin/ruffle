import unittest
import tempfile
from pathlib import Path

from summarize import MIB, growth_per_minute, later_half, load_session, render, stats
import csv
import json


class SummaryTests(unittest.TestCase):
    def test_tail_uses_time_and_preserves_missing_samples(self):
        rows = [
            {"route_ms": 0, "private_commit_bytes": 100 * MIB},
            {"route_ms": 11000, "private_commit_bytes": 20 * MIB},
            {"route_ms": 12000, "private_commit_bytes": None},
            {"route_ms": 21000, "private_commit_bytes": 30 * MIB},
        ]
        result = stats(rows, "private_commit_bytes")
        self.assertEqual(result["tail"], 25)
        self.assertEqual(result["peak"], 100)
        self.assertEqual(result["coverage"], "3/4")

    def test_missing_gpu_counter_is_not_zero(self):
        self.assertIsNone(stats([{"route_ms": 0, "gpu_shared_bytes": None}], "gpu_shared_bytes"))
        self.assertEqual(stats([{"route_ms": 0, "gpu_shared_bytes": 0}], "gpu_shared_bytes")["tail"], 0)

    def test_comparison_rejects_different_route_and_incomplete_run(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "timeline.csv").write_text("event,session_ms,checkpoint\n", encoding="utf-8")
            first = {
                "path": path,
                "result": {"complete": "True", "outcome": "replay_finished", "scene_validation": "unreviewed", "max_dispatch_lateness_ms": "10"},
                "build": {"sha256": "a", "mode": "replay"},
                "recording": {"Events": []},
                "rows": [],
            }
            second = dict(first, recording={"Events": [{"Kind": "checkpoint"}]})
            self.assertIn("Comparison withheld", render([first, second]))
            second["recording"] = first["recording"]
            second["result"] = dict(first["result"], complete="False")
            self.assertIn("Comparison withheld", render([first, second]))
            second["result"] = first["result"]
            self.assertIn("Matched route segments", render([first, second]))

    def test_latter_half_trend_ignores_initial_asset_growth(self):
        rows = [{"route_ms": time * 60000, "private_commit_bytes": value * MIB}
                for time, value in [(0, 10), (1, 100), (2, 200), (3, 204), (4, 208)]]
        self.assertEqual(growth_per_minute(later_half(rows), "private_commit_bytes"), 4)
        self.assertIsNone(growth_per_minute(rows[:1], "private_commit_bytes"))

    def test_observation_loads_without_input_recording_and_withholds_matched_comparison(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "build.txt").write_text("pid=123\nmode=observe\nsha256=test\n", encoding="utf-8")
            (path / "result.txt").write_text("complete=True\noutcome=observation_finished\nscene_validation=manual_observation\nmax_dispatch_lateness_ms=0\nsampler_error=\n", encoding="utf-8")
            (path / "session.json").write_text(json.dumps({"Events": [], "Complete": False}), encoding="utf-8")
            (path / "timeline.csv").write_text("event,session_ms,checkpoint\nbattle_end,150000,2\n", encoding="utf-8")
            fields = ["session_ms", "route_ms", "phase", "checkpoint", "pid", "sample_duration_ms", "private_commit_bytes", "working_set_bytes", "private_working_set_bytes", "gpu_dedicated_bytes", "gpu_shared_bytes"]
            with (path / "memory.csv").open("w", encoding="utf-8", newline="") as stream:
                writer = csv.DictWriter(stream, fieldnames=fields)
                writer.writeheader()
                for index, value in [(0, 100), (2, 200), (3, 150)]:
                    writer.writerow(dict(session_ms=index * 60000, route_ms=index * 60000, phase="running", checkpoint=index, pid=123, sample_duration_ms=1, private_commit_bytes=value * MIB))
            session = load_session(path)
            report = render([session, session])
            self.assertIn("Marked battle endings: 1", report)
            self.assertIn("final 10 s medians: 50.0 MiB", report)
            self.assertIn("Comparison withheld", report)
            self.assertNotIn("## Matched route segments", report)
            # Background memory must not inflate foreground medians or produce
            # a slope spanning a period with a different game activity level.
            battle = next(row for row in session["rows"] if row["checkpoint"] == 2)
            session["rows"] = [dict(battle, route_ms=time * 60000, private_commit_bytes=value * MIB)
                               for time, value in [(0, 100), (2, 200), (4, 300)]]
            session["rows"].append(dict(battle, phase="background", route_ms=180000,
                                        private_commit_bytes=9000 * MIB))
            report = render([session])
            self.assertIn("Background samples: 1", report)
            self.assertIn("| Continuous battles | 240.0 | 3 | 200.0 | 250.0 | N/A |", report)


if __name__ == "__main__":
    unittest.main()
