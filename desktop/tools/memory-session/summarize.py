#!/usr/bin/env python3
"""Summarize one Windows memory session, or compare two identical input routes."""

import argparse
import csv
import json
import math
import statistics
from pathlib import Path

METRICS = {
    "private_commit_bytes": "Private commit",
    "working_set_bytes": "Working set",
    "private_working_set_bytes": "Private working set",
    "gpu_dedicated_bytes": "GPU dedicated",
    "gpu_shared_bytes": "GPU shared",
}
MIB = 1024 * 1024


def read_properties(path):
    return dict(line.split("=", 1) for line in path.read_text(encoding="utf-8-sig").splitlines() if "=" in line)


def load_session(path):
    result = read_properties(path / "result.txt")
    build = read_properties(path / "build.txt")
    observe = build["mode"] == "observe"
    recording = json.loads((path / ("session.json" if observe else "recording.json")).read_text(encoding="utf-8-sig"))
    with (path / "memory.csv").open(encoding="utf-8-sig", newline="") as stream:
        rows = list(csv.DictReader(stream))
    for row in rows:
        for name in ("session_ms", "route_ms", "checkpoint", "sample_duration_ms", *METRICS):
            raw = row[name]
            value = float(raw) if raw else None
            if value is not None and (not math.isfinite(value) or value < 0):
                raise ValueError(f"{path}: invalid {name}")
            row[name] = value
        if str(row["pid"]) != build["pid"]:
            raise ValueError(f"{path}: sample PID differs from build metadata")
    return {"path": path, "result": result, "build": build, "recording": recording, "rows": rows}


def stats(rows, metric):
    """Tail is the final ten seconds of route time, not the last ten samples."""
    if not rows:
        return None
    values = [row[metric] for row in rows if row[metric] is not None]
    if not values:
        return None
    end = max(row["route_ms"] for row in rows)
    tail = [row[metric] for row in rows if row["route_ms"] >= end - 10000 and row[metric] is not None]
    return {
        "median": statistics.median(values) / MIB,
        "peak": max(values) / MIB,
        "tail": statistics.median(tail) / MIB if tail else None,
        "coverage": f"{len(values)}/{len(rows)}",
    }


def fmt(value):
    return "N/A" if value is None else f"{value:.1f}"


def running_rows(session, checkpoint=None):
    return [row for row in session["rows"] if row["phase"] == "running" and
            (checkpoint is None or row["checkpoint"] == checkpoint)]


def later_half(rows):
    if not rows:
        return []
    middle = (min(row["route_ms"] for row in rows) + max(row["route_ms"] for row in rows)) / 2
    return [row for row in rows if row["route_ms"] >= middle]


def growth_per_minute(rows, metric):
    """OLS trend in MiB/min; refuse less than one minute of measured coverage."""
    points = [(row["route_ms"] / 60000, row[metric] / MIB) for row in rows if row[metric] is not None]
    if len(points) < 3 or max(x for x, _ in points) - min(x for x, _ in points) < 1:
        return None
    x_mean = statistics.mean(x for x, _ in points)
    y_mean = statistics.mean(y for _, y in points)
    return sum((x - x_mean) * (y - y_mean) for x, y in points) / sum((x - x_mean) ** 2 for x, _ in points)


def observation_report(session):
    lines = ["### Continuous battle observation", "",
             "Stages are manual F9 markers, not game-state detection. F7 counts only marked battle endings; omitted marks are not inferred.",
             "Growth is an ordinary least-squares trend over the latter half of each stage (minimum 60 s coverage), not evidence of a leak.", "",
             "| Stage | Sampled span (s) | Samples | Private commit median MiB | Latter-half median MiB | Latter-half MiB/min |",
             "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for index, label in enumerate(("Initial idle", "Map loading", "Continuous battles", "Cooldown")):
        rows = running_rows(session, index)
        whole = stats(rows, "private_commit_bytes")
        latter = later_half(rows)
        stable = stats(latter, "private_commit_bytes")
        span = (max(row["route_ms"] for row in rows) - min(row["route_ms"] for row in rows)) / 1000 if rows else None
        lines.append(f"| {label} | {fmt(span)} | {len(rows)} | {fmt(whole['median'] if whole else None)} | "
                     f"{fmt(stable['median'] if stable else None)} | {fmt(growth_per_minute(latter, 'private_commit_bytes'))} |")
    with (session["path"] / "timeline.csv").open(encoding="utf-8-sig", newline="") as stream:
        marked = sum(row["event"] == "battle_end" for row in csv.DictReader(stream))
    initial = stats(running_rows(session, 0), "private_commit_bytes")
    cooldown = stats(running_rows(session, 3), "private_commit_bytes")
    delta = cooldown["tail"] - initial["tail"] if initial and cooldown and initial["tail"] is not None and cooldown["tail"] is not None else None
    lines += ["", f"Marked battle endings: {marked} (may be partial).",
              f"Cooldown minus initial idle, final 10 s medians: {fmt(delta)} MiB.",
              "This retained difference includes loaded game resources and allocator retention; it is not automatically leaked memory.", ""]
    return lines


def render(sessions):
    lines = ["# Windows memory session report", "", "All memory values are MiB (1 MiB = 1,048,576 bytes).",
             "Private commit is the primary process metric. Working sets and GPU counters are separate; do not add these columns.",
             "Only running samples enter the tables; armed/checkpoint waits are excluded. Missing counters are N/A, never zero.", ""]
    for index, session in enumerate(sessions):
        result, build = session["result"], session["build"]
        lines += [f"## {'AB'[index]}: {session['path'].name}", "",
                  f"- Build SHA256: `{build['sha256']}`",
                  f"- Mode: {build['mode']}; outcome: {result['outcome']}; complete: {result['complete']}",
                  f"- Scene validation: {result['scene_validation']}; maximum dispatch delay: {result['max_dispatch_lateness_ms']} ms",
                  f"- Running samples: {len(running_rows(session))}; total samples: {len(session['rows'])}", ""]
        if result["complete"].lower() != "true" or result.get("sampler_error"):
            lines += ["**Invalid/incomplete run: do not use this run to claim a memory improvement.**", ""]
        if result["scene_validation"] == "unreviewed":
            lines += ["**Unattended replay: review checkpoint screenshots before accepting this comparison.**", ""]
        lines += ["| Metric | Median | Peak | Last 10 s median | Available samples |",
                  "| --- | ---: | ---: | ---: | ---: |"]
        for metric, label in METRICS.items():
            value = stats(running_rows(session), metric)
            cells = [fmt(value[k]) if value else "N/A" for k in ("median", "peak", "tail")]
            lines.append(f"| {label} | {' | '.join(cells)} | {value['coverage'] if value else '0'} |")
        lines.append("")
        if build["mode"] == "observe":
            lines += observation_report(session)

    if len(sessions) == 2:
        first, second = sessions
        valid = all(s["result"]["complete"].lower() == "true" and not s["result"].get("sampler_error") for s in sessions)
        same_route = all(s["build"]["mode"] != "observe" for s in sessions) and first["recording"] == second["recording"]
        if not same_route or not valid:
            lines += ["## Comparison withheld", "",
                      "Matched-input comparison requires two completed runs without sampling errors and the identical input recording (including dimensions and DPI). Observations have no replayable inputs; compare their stage reports and workload notes manually.", ""]
        else:
            lines += ["## Matched route segments", "",
                      "Segment 0 starts at F8; segment N starts after checkpoint N. B minus A uses each segment's final 10 seconds median.",
                      "Checkpoint pauses can allow additional loading/GC; also compare wait durations below and inspect screenshots.", "",
                      "| Segment | Metric | A tail MiB | B tail MiB | B - A MiB | A / B samples |",
                      "| --- | --- | ---: | ---: | ---: | ---: |"]
            count = sum(e["Kind"] == "checkpoint" for e in first["recording"]["Events"])
            for segment in range(count + 1):
                a_rows, b_rows = (running_rows(s, segment) for s in sessions)
                for metric, label in METRICS.items():
                    a, b = stats(a_rows, metric), stats(b_rows, metric)
                    a_tail, b_tail = a["tail"] if a else None, b["tail"] if b else None
                    delta = b_tail - a_tail if a_tail is not None and b_tail is not None else None
                    lines.append(f"| {segment} | {label} | {fmt(a_tail)} | {fmt(b_tail)} | {fmt(delta)} | {len(a_rows)} / {len(b_rows)} |")
            lines.append("")
    lines += ["## Checkpoint waits", "", "| Run | Checkpoint | Wait seconds |", "| --- | ---: | ---: |"]
    for index, session in enumerate(sessions):
        with (session["path"] / "timeline.csv").open(encoding="utf-8-sig", newline="") as stream:
            start = None
            for row in csv.DictReader(stream):
                if row["event"] == "checkpoint":
                    start = row
                elif row["event"] == "resume" and start:
                    seconds = (float(row["session_ms"]) - float(start["session_ms"])) / 1000
                    lines.append(f"| {'AB'[index]} | {start['checkpoint']} | {seconds:.2f} |")
                    start = None
    lines += ["", "Identical input does not establish identical server responses, game state, or loaded assets. This report measures no leak by itself.", ""]
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("sessions", type=Path, nargs="+")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if not 1 <= len(args.sessions) <= 2:
        parser.error("Supply one session, or two sessions for comparison.")
    report = render([load_session(path) for path in args.sessions])
    if args.output:
        args.output.write_text(report, encoding="utf-8")
    else:
        print(report)


if __name__ == "__main__":
    main()
