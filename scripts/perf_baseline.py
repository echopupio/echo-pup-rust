#!/usr/bin/env python3
"""Aggregate EchoPup latency logs into P50/P95 baseline stats."""

from __future__ import annotations

import argparse
import csv
import json
import math
import os
import platform
import re
import sys
from dataclasses import asdict, dataclass
from datetime import datetime
from typing import Dict, Iterable, List, Optional

LINE_PATTERN = re.compile(
    r"\[(?P<trigger>[^\]]+)\]\s+性能埋点:\s+"
    r"stt_ms=(?P<stt_ms>\d+)\s+"
    r"llm_ms=(?P<llm_ms>\d+)\s+"
    r"postprocess_ms=(?P<postprocess_ms>\d+)\s+"
    r"type_ms=(?P<type_ms>\d+)\s+"
    r"e2e_ms=(?P<e2e_ms>\d+)"
)

METRICS = ["stt_ms", "llm_ms", "postprocess_ms", "type_ms", "e2e_ms"]


@dataclass
class PerfRecord:
    trigger: str
    stt_ms: int
    llm_ms: int
    postprocess_ms: int
    type_ms: int
    e2e_ms: int


@dataclass
class MetricSummary:
    p50: float
    p95: float
    avg: float
    max: int


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Summarize EchoPup latency logs and export baseline rows."
    )
    parser.add_argument(
        "--log",
        default="~/.echopup/echopup.log",
        help="Path to echopup log file (default: ~/.echopup/echopup.log)",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=200,
        help="Use only the latest N latency records (default: 200)",
    )
    parser.add_argument(
        "--machine",
        default=f"{platform.system()}-{platform.machine()}",
        help="Machine label for baseline export",
    )
    parser.add_argument(
        "--profile",
        default="unknown",
        help="Performance profile label for baseline export (accurate/balanced/fast/...)",
    )
    parser.add_argument(
        "--export-csv",
        help="Append one baseline row to CSV (creates file and header if needed)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Output JSON summary instead of table text",
    )
    return parser.parse_args()


def percentile(sorted_values: List[int], q: float) -> float:
    if not sorted_values:
        raise ValueError("empty values")
    if len(sorted_values) == 1:
        return float(sorted_values[0])
    rank = (len(sorted_values) - 1) * (q / 100.0)
    lo = int(math.floor(rank))
    hi = int(math.ceil(rank))
    if lo == hi:
        return float(sorted_values[lo])
    weight = rank - lo
    return sorted_values[lo] * (1.0 - weight) + sorted_values[hi] * weight


def parse_records(lines: Iterable[str]) -> List[PerfRecord]:
    records: List[PerfRecord] = []
    for line in lines:
        match = LINE_PATTERN.search(line)
        if not match:
            continue
        records.append(
            PerfRecord(
                trigger=match.group("trigger"),
                stt_ms=int(match.group("stt_ms")),
                llm_ms=int(match.group("llm_ms")),
                postprocess_ms=int(match.group("postprocess_ms")),
                type_ms=int(match.group("type_ms")),
                e2e_ms=int(match.group("e2e_ms")),
            )
        )
    return records


def summarize(records: List[PerfRecord]) -> Dict[str, object]:
    if not records:
        raise ValueError("no records")

    summary: Dict[str, object] = {
        "count": len(records),
        "llm_hit_rate": sum(1 for r in records if r.llm_ms > 0) / len(records),
        "metrics": {},
    }
    metrics_summary: Dict[str, Dict[str, object]] = {}
    for metric in METRICS:
        values = sorted(getattr(r, metric) for r in records)
        metrics_summary[metric] = asdict(
            MetricSummary(
                p50=percentile(values, 50),
                p95=percentile(values, 95),
                avg=sum(values) / len(values),
                max=values[-1],
            )
        )
    summary["metrics"] = metrics_summary
    return summary


def grouped_summaries(records: List[PerfRecord]) -> Dict[str, Dict[str, object]]:
    groups: Dict[str, List[PerfRecord]] = {"overall": records}
    for record in records:
        groups.setdefault(record.trigger, []).append(record)
    return {name: summarize(group) for name, group in groups.items()}


def print_text_summary(
    summaries: Dict[str, Dict[str, object]], log_path: str, source_count: int, used_count: int
) -> None:
    print(f"log: {log_path}")
    print(f"records: {used_count}/{source_count} (latest window)")
    print("")
    for scope, summary in summaries.items():
        print(f"[{scope}] count={summary['count']} llm_hit_rate={summary['llm_hit_rate']:.1%}")
        print("metric          p50_ms   p95_ms   avg_ms   max_ms")
        metrics = summary["metrics"]
        for metric in METRICS:
            item = metrics[metric]
            print(
                f"{metric:<15} {item['p50']:>7.1f} {item['p95']:>8.1f} "
                f"{item['avg']:>8.1f} {item['max']:>8}"
            )
        print("")


def export_csv(
    path: str,
    machine: str,
    profile: str,
    log_path: str,
    summary: Dict[str, object],
) -> None:
    metrics = summary["metrics"]
    row = {
        "timestamp": datetime.now().isoformat(timespec="seconds"),
        "machine": machine,
        "profile": profile,
        "record_count": summary["count"],
        "llm_hit_rate": f"{summary['llm_hit_rate']:.4f}",
        "stt_p50_ms": f"{metrics['stt_ms']['p50']:.1f}",
        "stt_p95_ms": f"{metrics['stt_ms']['p95']:.1f}",
        "llm_p50_ms": f"{metrics['llm_ms']['p50']:.1f}",
        "llm_p95_ms": f"{metrics['llm_ms']['p95']:.1f}",
        "postprocess_p50_ms": f"{metrics['postprocess_ms']['p50']:.1f}",
        "postprocess_p95_ms": f"{metrics['postprocess_ms']['p95']:.1f}",
        "type_p50_ms": f"{metrics['type_ms']['p50']:.1f}",
        "type_p95_ms": f"{metrics['type_ms']['p95']:.1f}",
        "e2e_p50_ms": f"{metrics['e2e_ms']['p50']:.1f}",
        "e2e_p95_ms": f"{metrics['e2e_ms']['p95']:.1f}",
        "log_path": log_path,
    }

    parent = os.path.dirname(path)
    if parent:
        os.makedirs(parent, exist_ok=True)

    file_exists = os.path.exists(path)
    with open(path, "a", newline="", encoding="utf-8") as fh:
        writer = csv.DictWriter(fh, fieldnames=list(row.keys()))
        if not file_exists:
            writer.writeheader()
        writer.writerow(row)


def main() -> int:
    args = parse_args()
    log_path = os.path.expanduser(args.log)
    if not os.path.exists(log_path):
        print(f"error: log file not found: {log_path}", file=sys.stderr)
        return 1

    with open(log_path, "r", encoding="utf-8", errors="ignore") as fh:
        all_records = parse_records(fh)
    if not all_records:
        print("error: no latency records found in log", file=sys.stderr)
        return 2

    limit = max(1, args.limit)
    records = all_records[-limit:]
    summaries = grouped_summaries(records)

    if args.json:
        output = {
            "log_path": log_path,
            "source_count": len(all_records),
            "used_count": len(records),
            "summaries": summaries,
        }
        print(json.dumps(output, ensure_ascii=False, indent=2))
    else:
        print_text_summary(summaries, log_path, len(all_records), len(records))

    if args.export_csv:
        export_csv(args.export_csv, args.machine, args.profile, log_path, summaries["overall"])
        if not args.json:
            print(f"baseline appended: {args.export_csv}")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
