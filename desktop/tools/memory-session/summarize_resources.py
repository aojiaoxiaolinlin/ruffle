#!/usr/bin/env python3
"""Extract allowlisted numeric diagnostics and align them to OS memory samples."""

import argparse
from bisect import bisect_left
import csv
from datetime import datetime
from pathlib import Path
import re
import statistics

from summarize import MIB, load_session

STAGES = ("Idle", "Maps", "Battles", "Cooldown")
CORE = set("gc_objects gc_debt known_movies swf_data characters graphic_mesh_refs morph_frames morph_mesh_refs bitmap_fill_refs font_lookup_entries font_sort_entries default_font_entries".split())
GPU = set("shapes shape_vertex_index uploaded_count target_count cache_count scratch_owned scratch_checked_out scratch_peak shape_registry_entries shape_registry_capacity_bytes".split())
NATIVE = {kind + suffix for kind in ("script_slots", "bytearray_capacity", "bitmap_pixels", "vtable_tables", "vtable_traits_lower_bound", "vtable_metadata_lower_bound", "dynamic_map_tables", "vector_capacity")
          for suffix in ("_live", "_created", "_dropped", "_bytes")}
KINDS = {"core-memory:": CORE, "native-memory:": NATIVE, "gpu-memory:": GPU,
         "GPU allocator bytes:": {"allocated", "reserved", "blocks", "allocations"},
         "CPU BitmapHandle pixel estimates:": {"uploaded", "targets", "cacheAsBitmap"},
         "SWF render cache sweep:": {"idle_seconds", "movies", "characters", "released_meshes", "released_morph_frames", "released_bitmap_fills"},
         "SWF render cache policy:": {"idle_seconds"}}
TIMESTAMP = re.compile(r"\b\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?(?:Z|[+-]\d\d:\d\d)")
NUMBER = re.compile(r"\b([a-zA-Z_]+)=([0-9]+(?:\.[0-9]+)?)(?:\s*(MiB))?\b")


def timestamp(value):
    return datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()


def extract(line):
    """Ignore arbitrary game log contents, including URLs and script traces."""
    for marker, allowed in KINDS.items():
        if marker not in line:
            continue
        prefix, payload = line.split(marker, 1)
        match = TIMESTAMP.search(prefix)
        if not match:
            return []
        rows = []
        for key, raw, unit in NUMBER.findall(payload.split(";", 1)[0]):
            if key not in allowed:
                continue
            is_bytes = key.endswith("_bytes") or (marker == "GPU allocator bytes:" and key in {"allocated", "reserved"})
            value = float(raw)
            if is_bytes:
                value /= MIB
            rows.append({"utc": match.group(), "kind": marker.rstrip(":"), "metric": key,
                         "value": value, "unit": "seconds" if key == "idle_seconds" else "debt units" if key == "gc_debt" else "MiB" if unit or is_bytes else "count",
                         "semantics": "interval" if marker == "SWF render cache sweep:" else "cumulative" if key.endswith(("_created", "_dropped")) else "gauge"})
        return rows
    return []


def align(rows, session, tolerance=2):
    samples = sorted((timestamp(row["utc"]), index) for index, row in enumerate(session["rows"]))
    times = [time for time, _ in samples]
    for row in rows:
        time = timestamp(row["utc"])
        pos = bisect_left(times, time)
        candidates = [index for index in (pos - 1, pos) if 0 <= index < len(samples)]
        row.update(phase="unaligned", stage="", sample_offset_s="", private_commit_mib="", working_set_mib="", gpu_dedicated_mib="")
        if not candidates:
            continue
        closest = min(candidates, key=lambda index: abs(times[index] - time))
        if abs(times[closest] - time) > tolerance:
            continue
        sample = session["rows"][samples[closest][1]]
        row.update(phase=sample["phase"], stage=int(sample["checkpoint"]), sample_offset_s=times[closest] - time)
        for source, target in (("private_commit_bytes", "private_commit_mib"), ("working_set_bytes", "working_set_mib"), ("gpu_dedicated_bytes", "gpu_dedicated_mib")):
            row[target] = sample[source] / MIB if sample[source] is not None else ""
    return rows


def render(rows):
    lines = ["# Resource attribution diagnostics", "",
             "Resource records are matched to the nearest OS sample within 2 seconds; sources are not atomic snapshots.",
             "Only foreground running records enter the table. First/last values are medians within the first/last 10 seconds of each metric in each stage.",
             "Cumulative creation/drop counts are activity totals, not retained objects or bytes. Gauges may include unswept garbage.",
             "Cache sweep records describe each scan's activity, not live resources or cumulative totals.",
             "Metrics overlap and cover only selected resources; do not add them or label unexplained process memory as a leak.",
             "Different metric endpoints may differ; inspect resources.csv and timeline.csv before comparing changes.", "",
             "| Stage | Source | Metric | Unit | Type | Records | First | Last | Change |",
             "| --- | --- | --- | --- | --- | ---: | ---: | ---: | ---: |"]
    groups = {}
    for row in rows:
        if row["phase"] == "running":
            groups.setdefault((row["stage"], row["kind"], row["metric"], row["unit"], row["semantics"]), []).append(row)
    for (stage, kind, metric, unit, semantics), records in sorted(groups.items()):
        start = min(timestamp(row["utc"]) for row in records)
        end = max(timestamp(row["utc"]) for row in records)
        first = statistics.median(row["value"] for row in records if timestamp(row["utc"]) <= start + 10)
        last = statistics.median(row["value"] for row in records if timestamp(row["utc"]) >= end - 10)
        lines.append(f"| {STAGES[stage]} | {kind} | {metric} | {unit} | {semantics} | {len(records)} | {first:.2f} | {last:.2f} | {last-first:+.2f} |")
    present = {row["kind"] for row in rows}
    missing = [marker.rstrip(":") for marker in KINDS if not marker.startswith("SWF render cache") and marker.rstrip(":") not in present]
    if missing:
        lines += ["", "Missing diagnostic categories: " + ", ".join(missing) + ". Absence is not zero usage; allocator reporting is backend-dependent."]
    if not groups:
        lines += ["", "No resource records aligned to foreground samples. Verify build features, RUST_LOG, timestamps and log paths."]
    lines += ["", f"Extracted numeric records: {len(rows)}; unaligned: {sum(row['phase'] == 'unaligned' for row in rows)}.", ""]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("session", type=Path, help="Memory sampling directory containing memory.csv")
    parser.add_argument("--log", type=Path, action="append", help="Explicit log path; repeat for multiple files")
    args = parser.parse_args()
    logs = args.log or sorted((args.session.parent / "game-cache" / "log").glob("*.log"))
    if not logs:
        parser.error("No game logs found. Specify --log or use Start-DiagnosticSession.ps1.")
    rows = []
    for path in logs:
        with path.open(encoding="utf-8-sig", errors="replace") as stream:
            for line in stream:
                rows.extend(extract(line))
    align(rows, load_session(args.session))
    fields = ["utc", "kind", "metric", "value", "unit", "semantics", "phase", "stage", "sample_offset_s", "private_commit_mib", "working_set_mib", "gpu_dedicated_mib"]
    with (args.session / "resources.csv").open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        writer.writerows(rows)
    report = args.session / "resource-report.md"
    report.write_text(render(rows), encoding="utf-8")
    print(report)


if __name__ == "__main__":
    main()
