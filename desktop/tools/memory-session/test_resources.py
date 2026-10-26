import unittest

from summarize import MIB
from summarize_resources import align, extract, render


class ResourceTests(unittest.TestCase):
    def test_dynamic_tables_are_gauges_and_debt_has_no_byte_unit(self):
        rows = extract("2026-10-26T14:00:00Z native-memory: dynamic_map_tables_bytes=1048576 dynamic_map_tables_created=12 dynamic_map_tables_dropped=3")
        self.assertEqual([(r["value"], r["unit"], r["semantics"]) for r in rows],
                         [(1, "MiB", "gauge"), (12, "count", "cumulative"), (3, "count", "cumulative")])

    def test_cache_sweeps_are_activity_counts_not_live_memory(self):
        rows = extract("2026-10-26T14:00:00Z SWF render cache sweep: idle_seconds=90 movies=100 characters=3000 released_meshes=12 released_morph_frames=3 released_bitmap_fills=2; per-sweep releases")
        self.assertEqual(rows[0]["unit"], "seconds")
        self.assertTrue(all(row["semantics"] == "interval" for row in rows))
        self.assertEqual(next(row["value"] for row in rows if row["metric"] == "released_meshes"), 12)

    def test_only_allowlisted_numbers_are_extracted_with_units_and_semantics(self):
        rows = extract("2026-10-26T14:00:00Z INFO native-memory: scope=process bitmap_pixels_bytes=2097152 bitmap_pixels_created=9 bitmap_pixels_dropped=3 secret=123; bitmap_pixels_bytes=999")
        self.assertEqual([(row["metric"], row["value"], row["unit"], row["semantics"]) for row in rows], [
            ("bitmap_pixels_bytes", 2, "MiB", "gauge"),
            ("bitmap_pixels_created", 9, "count", "cumulative"),
            ("bitmap_pixels_dropped", 3, "count", "cumulative"),
        ])
        self.assertEqual(extract("2026-10-26T14:00:00Z INFO avm_trace: secret=123"), [])
        self.assertEqual(extract("no timestamp gpu-memory: shapes=123"), [])

    def test_gpu_and_core_mib_are_not_converted_twice(self):
        rows = extract("2026-10-26T14:00:00.123456Z INFO gpu-memory: shapes=3 shape_vertex_index=12.5MiB shape_registry_capacity_bytes=1048576")
        self.assertEqual([(row["value"], row["unit"]) for row in rows], [(3, "count"), (12.5, "MiB"), (1, "MiB")])
        self.assertEqual(extract("2026-10-26T14:00:00Z core-memory: swf_data=42.5MiB")[0]["value"], 42.5)
        self.assertEqual(extract("2026-10-26T14:00:00Z core-memory: gc_debt=42")[0]["unit"], "debt units")
        self.assertEqual(extract("2026-10-26T14:00:00Z GPU allocator bytes: allocated=1048576 reserved=2097152")[1]["value"], 2)
        pixels = extract("2026-10-26T14:00:00Z CPU BitmapHandle pixel estimates: uploaded=12.5 MiB (10), targets=1.0 MiB (2), cacheAsBitmap=3.0 MiB (4); excludes GPU-only")
        self.assertEqual([(row["metric"], row["value"], row["unit"]) for row in pixels],
                         [("uploaded", 12.5, "MiB"), ("targets", 1, "MiB"), ("cacheAsBitmap", 3, "MiB")])

    def test_alignment_preserves_background_and_rejects_distant_samples(self):
        session = {"rows": [dict(utc="2026-10-26T14:00:00Z", phase="background", checkpoint=2,
                                private_commit_bytes=2*MIB, working_set_bytes=MIB, gpu_dedicated_bytes=None)]}
        rows = extract("2026-10-26T14:00:01Z gpu-memory: shapes=3")
        rows += extract("2026-10-26T14:00:10Z gpu-memory: shapes=4")
        align(rows, session)
        self.assertEqual(rows[0]["phase"], "background")
        self.assertEqual(rows[0]["private_commit_mib"], 2)
        self.assertEqual(rows[0]["gpu_dedicated_mib"], "")
        self.assertEqual(rows[1]["phase"], "unaligned")
        self.assertIn("No resource records aligned to foreground", render(rows))

    def test_report_compares_gauges_separately_from_creation_totals(self):
        rows = []
        for second, count in [(0, 10), (5, 12), (30, 20), (35, 22)]:
            record = extract(f"2026-10-26T14:00:{second:02}Z native-memory: bitmap_pixels_live={count} bitmap_pixels_created={count*2}")
            for row in record:
                row.update(phase="running", stage=2)
            rows.extend(record)
        report = render(rows)
        self.assertIn("bitmap_pixels_live | count | gauge | 4 | 11.00 | 21.00 | +10.00", report)
        self.assertIn("bitmap_pixels_created | count | cumulative | 4 | 22.00 | 42.00 | +20.00", report)


if __name__ == "__main__":
    unittest.main()
