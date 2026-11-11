#!/usr/bin/env python3
"""macOS resource benchmark: benchmark.py STOCK_BIN PATCHED_BIN OUTPUT_DIR."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("stock", type=Path)
parser.add_argument("patched", type=Path)
parser.add_argument("output", type=Path)
parser.add_argument("--repeats", type=int, default=3)
parser.add_argument("--threads", type=int, default=14)
args = parser.parse_args()
if platform.system() != "Darwin":
    parser.error("this recorded benchmark uses macOS time -l; the fixture checker also supports Linux")
if args.repeats < 1 or args.threads < 1:
    parser.error("repeat and thread counts must be positive")
root = Path(__file__).resolve().parents[3]
check = Path(__file__).resolve().parent
fixtures = root / "vendor/low-memory-prover/check/fixtures"
binaries = {"stock": args.stock.resolve(), "patched": args.patched.resolve()}
args.output.mkdir(parents=True, exist_ok=True)
goldens = dict((name, digest) for digest, name in (line.split() for line in (check / "expected.sha256").read_text().splitlines()))
metadata = {"platform": platform.platform(), "threads": args.threads, "repeats": args.repeats,
            "binary_sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in binaries.items()}}
(args.output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
columns = ["fixture", "variant", "iteration", "threads", "setup_s", "prove_s", "wall_s", "user_s", "sys_s", "max_rss_bytes", "peak_footprint_bytes", "sha256"]
with (args.output / "measurements.csv").open("w", newline="") as stream:
    writer = csv.DictWriter(stream, fieldnames=columns)
    writer.writeheader()
    stream.flush()
    for fixture in ["transfer", "withdraw"]:
        for iteration in range(1, args.repeats + 1):
            order = ["stock", "patched"] if iteration % 2 else ["patched", "stock"]
            for variant in order:
                label = f"{fixture}-{variant}-{iteration}"
                proof, log = (args.output / f"{label}.{suffix}" for suffix in ["proof", "log"])
                with log.open("w") as stderr:
                    subprocess.run(["/usr/bin/time", "-l", str(binaries[variant]), str(fixtures / f"strk20-{fixture}.pie.zip"), str(proof)],
                                   env={**os.environ, "RAYON_NUM_THREADS": str(args.threads)}, stdout=subprocess.DEVNULL, stderr=stderr, check=True)
                text = log.read_text()
                timing = re.search(r"setup ([\d.]+)s prove ([\d.]+)s", text)
                system = re.search(r"([\d.]+) real\s+([\d.]+) user\s+([\d.]+) sys", text)
                digest = hashlib.sha256(proof.read_bytes()).hexdigest()
                if digest != goldens[f"strk20-{fixture}"]:
                    raise RuntimeError(f"{label}: V2 proof byte mismatch")
                row = dict(zip(columns, [fixture, variant, iteration, args.threads, *timing.groups(), *system.groups(),
                           re.search(r"(\d+)\s+maximum resident set size", text).group(1),
                           re.search(r"(\d+)\s+peak memory footprint", text).group(1), digest]))
                writer.writerow(row)
                stream.flush()
                print(json.dumps(row), flush=True)
