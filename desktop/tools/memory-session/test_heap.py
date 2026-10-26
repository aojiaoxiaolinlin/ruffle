import unittest
from summarize_heap import parse_export, verify_fixture


def record(instance, site="transient_allocation_site", sizes=(262144,), total=None):
    return (f"PID [000123] Instance [ {instance}], [64bit] ====\n"
            f"  StackReference : abc, identical references = {len(sizes)}, StackID : 0\n"
            f"\t\truffle_heap_fixture.exe ruffle_heap_fixture::{site}\n"
            f"  Allocation size(s) : {', '.join(map(str, sizes))},\n"
            f"  Total Allocation of this stack: {sum(sizes) if total is None else total}\n")


class HeapTests(unittest.TestCase):
    def test_instances_are_separate_and_trailing_catalog_is_not_counted(self):
        text = "Total heap snapshot instances : 2\n" + record(1) + record(2)
        text += " Stack ID : 0\n\truffle_heap_fixture.exe transient_allocation_site\n"
        rows = parse_export(text)
        self.assertEqual([r["total_bytes"] for r in rows], [262144, 262144])
        self.assertEqual([r["allocation_count"] for r in rows], [1, 1])

    def test_inconsistent_size_and_incomplete_export_are_rejected(self):
        with self.assertRaises(ValueError):
            parse_export("Total heap snapshot instances : 1\n" + record(1, total=99))
        with self.assertRaises(ValueError):
            parse_export("Total heap snapshot instances : 2\n" + record(1))

    def test_count_mismatch_is_rejected(self):
        with self.assertRaises(ValueError):
            parse_export("Total heap snapshot instances : 1\nInstance 1 allocation count = 5\n" + record(1))

    def test_empty_snapshots_do_not_pass_fixture_symbol_verification(self):
        text = "Total heap snapshot instances : 3\n" + "".join(record(i, site="0x1234", sizes=(24,)) for i in (1, 2, 3))
        with self.assertRaises(ValueError):
            verify_fixture(parse_export(text))


if __name__ == "__main__":
    unittest.main()
