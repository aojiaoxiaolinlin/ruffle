"""Validate and summarize xperf -a heapsnapshot -instance -data exports.

Snapshot stacks describe allocations still held at the snapshot, not all past
allocation activity. Counts/bytes are never summed across different snapshots.
"""
import argparse
import json
from pathlib import Path
import re

INSTANCE = re.compile(r"PID\s*\[(\d+)\]\s*Instance\s*\[\s*(\d+)\]")
STACK = re.compile(r"StackReference\s*:\s*([0-9a-fA-F]+).*?identical references\s*=\s*(\d+).*?StackID\s*:\s*(\d+)")
COUNT = re.compile(r"Instance\s+(\d+)\s+allocation count\s*=\s*(\d+)")
BLOCK = 256 * 1024
SITES = ("retained_allocation_site", "transient_allocation_site")


def parse_export(text):
    instances = []
    current = None
    stack = None
    declared_counts = dict((int(a), int(b)) for a, b in COUNT.findall(text))
    declared_instances = re.search(r"Total heap snapshot instances\s*:\s*(\d+)", text)

    def finish_stack():
        nonlocal stack
        if stack is None:
            return
        if "total_bytes" not in stack or "sizes" not in stack:
            raise ValueError("Incomplete allocation stack record")
        if len(stack["sizes"]) != stack["references"] or sum(stack["sizes"]) != stack["total_bytes"]:
            raise ValueError("Allocation sizes disagree with stack count/total")
        current["stacks"].append(stack)
        stack = None

    for line in text.splitlines():
        match = INSTANCE.search(line)
        if match:
            finish_stack()
            current = {"pid": int(match[1]), "instance": int(match[2]), "stacks": []}
            instances.append(current)
        elif line.lstrip().startswith("Stack ID :"):
            # Optional trailing global stack catalog is not allocation data.
            finish_stack()
            current = None
        elif current is not None:
            match = STACK.search(line)
            if match:
                finish_stack()
                stack = {"reference": match[1], "references": int(match[2]),
                         "stack_id": int(match[3]), "frames": []}
            elif stack is not None:
                if "Allocation size(s) :" in line:
                    stack["sizes"] = [int(n) for n in re.findall(r"\d+", line.split(":", 1)[1])]
                elif "Total Allocation of this stack:" in line:
                    stack["total_bytes"] = int(line.rsplit(":", 1)[1].strip())
                elif line.startswith("\t\t"):
                    stack["frames"].append(line.strip())
    finish_stack()
    if not instances or declared_instances is None or len(instances) != int(declared_instances[1]):
        raise ValueError("Missing or incomplete snapshot instances")
    for instance in instances:
        stacks = instance["stacks"]
        instance["allocation_count"] = sum(s["references"] for s in stacks)
        instance["total_bytes"] = sum(s["total_bytes"] for s in stacks)
        if instance["instance"] in declared_counts and instance["allocation_count"] != declared_counts[instance["instance"]]:
            raise ValueError("Snapshot allocation count disagrees with export header")
    return instances


def fixture_rows(instances):
    rows = []
    for instance in instances:
        row = {"instance": instance["instance"], "pid": instance["pid"],
               "total_bytes": instance["total_bytes"], "allocation_count": instance["allocation_count"]}
        for site in SITES:
            matching = [s for s in instance["stacks"] if any(
                "ruffle_heap_fixture::" + site in frame for frame in s["frames"])]
            row[site] = sum(size for s in matching for size in s["sizes"] if size == BLOCK)
            row[site + "_blocks"] = sum(size == BLOCK for s in matching for size in s["sizes"])
        rows.append(row)
    return rows


def verify_fixture(instances):
    rows = fixture_rows(instances)
    if len(rows) != 3 or len({r["pid"] for r in rows}) != 1:
        raise ValueError("Fixture requires three snapshots of one PID")
    observed = [(r[SITES[0] + "_blocks"], r[SITES[1] + "_blocks"]) for r in rows]
    if observed != [(0, 0), (8, 64), (8, 0)]:
        raise ValueError(f"Fixture payload or symbol attribution mismatch: {observed}")
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--verify-fixture", action="store_true")
    args = parser.parse_args()
    instances = parse_export((args.directory / "heap-export.txt").read_text(encoding="utf-8-sig"))
    (args.directory / "heap-summary.json").write_text(json.dumps(instances, indent=2), encoding="utf-8")
    if args.verify_fixture:
        rows = verify_fixture(instances)
        report = ["# Offline heap attribution: PASS", "",
                  "The trace resolves both fixture allocation sites and captures the expected retained payload.", "",
                  "| Snapshot | PID | Heap allocations | Heap bytes | Retained payload MiB | Transient payload MiB |",
                  "|---|---:|---:|---:|---:|---:|"]
        for label, row in zip(("Baseline", "Allocated", "Released"), rows):
            report.append(f"| {label} | {row['pid']} | {row['allocation_count']} | {row['total_bytes']} | "
                          f"{row[SITES[0]] / 1048576:.0f} | {row[SITES[1]] / 1048576:.0f} |")
        report += ["", "64 × 256 KiB (16 MiB) at transient_allocation_site disappear after release; "
                   "8 × 256 KiB (2 MiB) at retained_allocation_site remain.", "",
                   "Heap totals include runtime allocations and Vec metadata. Snapshot totals are not process commit, "
                   "GPU memory, or the entire VirtualAlloc address space. This verifies the offline collection and "
                   "function attribution pipeline; it does not identify the game's memory growth.", ""]
        (args.directory / "heap-analysis.md").write_text("\n".join(report), encoding="utf-8")
        print("PASS: fixture allocation stacks, 16 MiB release, and 2 MiB retention verified.")
    else:
        print(f"Parsed {len(instances)} heap snapshots; saved heap-summary.json.")


if __name__ == "__main__":
    main()
