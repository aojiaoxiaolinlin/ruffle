"""Stream large xperf heap exports and compare held bytes by full frame sequence."""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
import subprocess
import sys

from summarize_heap import INSTANCE, COUNT

REFERENCE = re.compile(r"StackReference\s*:\s*([0-9a-fA-F]+).*?identical references\s*=\s*(\d+)")


def compare_lines(lines):
    groups = {}
    snapshots = []
    expected_instances = None
    expected_counts = {}
    current = None
    stack = None

    def finish():
        nonlocal stack
        if stack is None:
            return
        sizes = stack.get("sizes")
        total = stack.get("total")
        if sizes is None or total is None or sum(sizes.values()) != stack["count"] or sum(k*v for k, v in sizes.items()) != total:
            raise ValueError(f"Incomplete/inconsistent heap stack data: references={stack['count']}, "
                             f"sizes_count={sum(sizes.values()) if sizes is not None else None}, "
                             f"sizes_bytes={sum(k*v for k, v in sizes.items()) if sizes is not None else None}, total={total}")
        key = tuple(stack["frames"])
        group = groups.setdefault(key, {})
        values = group.setdefault(current, {"bytes": 0, "count": 0, "sizes": Counter()})
        values["bytes"] += total
        values["count"] += stack["count"]
        values["sizes"].update(sizes)
        snapshots[current]["bytes"] += total
        snapshots[current]["count"] += stack["count"]
        snapshots[current]["stack_records"] += 1
        snapshots[current]["reference_warnings"] += stack["warning"]
        stack = None

    for line in lines:
        match = re.search(r"Total heap snapshot instances\s*:\s*(\d+)", line)
        if match:
            expected_instances = int(match[1])
        match = COUNT.search(line)
        if match:
            expected_counts[int(match[1])] = int(match[2])
        match = INSTANCE.search(line)
        if match:
            finish()
            snapshots.append({"pid": int(match[1]), "instance": int(match[2]), "bytes": 0, "count": 0,
                              "stack_records": 0, "reference_warnings": 0})
            current = len(snapshots) - 1
        elif line.lstrip().startswith("Stack ID :"):
            finish()
            current = None
        elif current is not None:
            match = REFERENCE.search(line)
            if match:
                finish()
                stack = {"count": int(match[2]), "frames": [], "warning": int("Warning!" in line)}
            elif stack is not None:
                if "Allocation size(s) :" in line:
                    payload = line.split(":", 1)[1].split("Warning!", 1)[0]
                    stack["sizes"] = Counter(int(m[0]) for m in re.finditer(r"\d+", payload))
                    stack["reading_sizes"] = True
                elif "Total Allocation of this stack:" in line:
                    stack["total"] = int(line.rsplit(":", 1)[1].strip())
                    stack["reading_sizes"] = False
                elif stack.get("reading_sizes") and re.match(r"^\s*\d", line):
                    payload = line.split("Warning!", 1)[0]
                    stack["sizes"].update(int(m[0]) for m in re.finditer(r"\d+", payload))
                elif line.startswith("\t\t"):
                    stack["frames"].append(sys.intern(line.strip()))
    finish()
    if not snapshots or len(snapshots) != expected_instances:
        raise ValueError("Missing snapshot instances")
    if len({s["pid"] for s in snapshots}) != 1:
        raise ValueError("Compare one process per export")
    for snapshot in snapshots:
        expected = expected_counts.get(snapshot["instance"])
        if expected is not None and expected != snapshot["count"]:
            raise ValueError("Snapshot allocation count mismatch")
    if len(snapshots) < 2:
        raise ValueError("At least two snapshots required")
    rows = []
    for frames, instances in groups.items():
        values = [instances.get(i, {"bytes": 0, "count": 0, "sizes": {}}) for i in range(len(snapshots))]
        rows.append({"frames": list(frames), "bytes": [v["bytes"] for v in values],
                     "counts": [v["count"] for v in values],
                     "delta_bytes": values[-1]["bytes"] - values[0]["bytes"],
                     "last_sizes": sorted(values[-1]["sizes"].items(), key=lambda p: p[0]*p[1], reverse=True)[:10]})
    rows.sort(key=lambda r: r["delta_bytes"], reverse=True)
    return {"snapshots": snapshots, "groups": rows}


def demangle(result, executable):
    frames = sorted({f for r in result["groups"] for f in r["frames"]})
    run = subprocess.run([str(executable)], input="\n".join(frames)+"\n", capture_output=True, text=True, check=True, encoding="utf-8")
    decoded = run.stdout.splitlines()
    if len(decoded) != len(frames):
        raise ValueError("Demangler returned unexpected line count")
    mapping = dict(zip(frames, decoded))
    for row in result["groups"]:
        row["frames"] = [mapping[f] for f in row["frames"]]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--demangler", type=Path)
    args = parser.parse_args()
    with (args.directory / "heap-export.txt").open(encoding="utf-8-sig") as stream:
        result = compare_lines(stream)
    if args.demangler:
        demangle(result, args.demangler)
    (args.directory / "heap-diff.json").write_text(json.dumps(result, ensure_ascii=False), encoding="utf-8")
    print(json.dumps({"snapshots": result["snapshots"], "stack_groups": len(result["groups"])}, ensure_ascii=False))
    for row in result["groups"][:15]:
        application = [f for f in row["frames"] if any(n in f for n in ("ruffle_core::", "ruffle_render", "wgpu", "gc_arena::"))]
        label = application[-1] if application else (row["frames"][-1] if row["frames"] else "unknown")
        print(f"delta={row['delta_bytes']/1048576:.2f} MiB last={row['bytes'][-1]/1048576:.2f} MiB {label[:450]}")


if __name__ == "__main__":
    main()
