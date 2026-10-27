import io
import unittest
from compare_heap import compare_lines
from test_heap import record


class HeapDiffTests(unittest.TestCase):
    def test_groups_frames_across_different_ids_without_double_counting_catalog(self):
        text = "Total heap snapshot instances : 2\n" + record(1, sizes=(100, 100))
        text += record(2, sizes=(100, 100, 100)).replace("StackID : 0", "StackID : 99")
        text += " Stack ID : 99\n\truffle_heap_fixture.exe ignored\n"
        result = compare_lines(io.StringIO(text))
        self.assertEqual(len(result["groups"]), 1)
        self.assertEqual(result["groups"][0]["bytes"], [200, 300])
        self.assertEqual(result["groups"][0]["delta_bytes"], 100)

    def test_release_is_negative_and_unrelated_equal_ids_do_not_merge(self):
        text = "Total heap snapshot instances : 2\n" + record(1, site="retained_allocation_site", sizes=(100,))
        text += record(2, site="transient_allocation_site", sizes=(50,))
        result = compare_lines(io.StringIO(text))
        self.assertEqual([r["delta_bytes"] for r in result["groups"]], [50, -100])

    def test_rejects_partial_data(self):
        text = "Total heap snapshot instances : 2\n" + record(1) + record(2, total=123)
        with self.assertRaises(ValueError):
            compare_lines(io.StringIO(text))

    def test_xperf_warning_header_keeps_actual_allocation_count_and_flags_warning(self):
        original = record(1, sizes=(100, 100))
        warning = original.replace('StackID : 0',
            "Warning! The stack's reference count(3) does not match with the number this reference actually have seen(2)\n    StackID : 0")
        result = compare_lines(io.StringIO("Total heap snapshot instances : 2\n" + warning + record(2, sizes=(100,))))
        self.assertEqual(result["snapshots"][0]["reference_warnings"], 1)
        self.assertEqual(result["groups"][0]["delta_bytes"], -100)

    def test_warning_numbers_on_wrapped_size_lines_are_not_counted_as_allocations(self):
        first = record(1, sizes=(100, 100)).replace('100, 100,',
            "100, Warning! reference count(3) actually seen(2)\n100,")
        text = "Total heap snapshot instances : 2\n" + first + record(2, sizes=(50,))
        result = compare_lines(io.StringIO(text))
        self.assertEqual(result["snapshots"][0]["bytes"], 200)
        self.assertEqual(result["groups"][0]["delta_bytes"], -150)


if __name__ == "__main__":
    unittest.main()
