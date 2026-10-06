"""Build the GitHub Pages site into ``_site/``.

    python3 site/build.py

Standard library only. Copies the static page, packs the unmodified ``qbm``
package plus the browser bridge into ``qbm.zip`` for Pyodide, and summarises
the shipped result tables into ``data/published.json`` so every published
number on the page is read from ``results/`` rather than transcribed.
"""
from __future__ import annotations

import collections
import csv
import json
import shutil
import statistics
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SITE = ROOT / "site"
OUT = ROOT / "_site"
METHOD_ORDER = ["exhaustive", "tabu", "QAOA p=2", "annealing", "greedy", "QAOA p=3", "QAOA p=1"]


def read(name: str) -> list[dict]:
    with (ROOT / "results" / name).open(newline="") as handle:
        return list(csv.DictReader(handle))


def headtohead() -> list[dict]:
    groups: dict[str, list[bool]] = collections.defaultdict(list)
    families: dict[str, str] = {}
    formulations: dict[str, str] = {}
    for row in read("headtohead.csv"):
        name = row["algorithm"]
        label = f"QAOA p={name[-1]}" if name.startswith("qaoa_p") else name
        groups[label].append(row["found_optimum"] == "True")
        families[label] = "quantum" if name.startswith("qaoa") else "classical"
        formulations[label] = row["formulation"]
    rows = [{"method": label, "family": families[label], "formulation": formulations[label],
             "instances": len(hits), "success_rate": sum(hits) / len(hits)} for label, hits in groups.items()]
    return sorted(rows, key=lambda row: METHOD_ORDER.index(row["method"]) if row["method"] in METHOD_ORDER else 99)


def constrained_mixer() -> list[dict]:
    groups: dict[tuple[str, int], list[float]] = collections.defaultdict(list)
    for row in read("constrained_mixer.csv"):
        groups[(row["ansatz"], int(row["depth"]))].append(float(row["lift_over_random"]))
    return [{"ansatz": ansatz, "depth": depth, "instances": len(lifts), "median_lift": statistics.median(lifts),
             "beats_random": sum(lift > 1 for lift in lifts)} for (ansatz, depth), lifts in sorted(groups.items())]


def penalty_landscape() -> list[dict]:
    groups: dict[int, list[dict]] = collections.defaultdict(list)
    for row in read("penalty_landscape.csv"):
        groups[int(row["n_inputs"])].append(row)
    return [{"n_inputs": n, "qubits": int(rows[0]["qubits"]),
             "penalty_dominance": statistics.mean(float(r["penalty_dominance"]) for r in rows),
             "useful_band_pct": statistics.mean(100 * float(r["useful_span"]) / float(r["span"]) for r in rows)}
            for n, rows in sorted(groups.items())]


def flagship_curve() -> list[dict]:
    return [{"budget": int(row["K"]), "certified": float(row["ilp_cov"]), "greedy": float(row["greedy_cov"])}
            for row in read("flagship_coverage_curve.csv")]


def main() -> None:
    if OUT.exists():
        shutil.rmtree(OUT)
    shutil.copytree(SITE / "static", OUT)
    (OUT / ".nojekyll").touch()

    with zipfile.ZipFile(OUT / "qbm.zip", "w", zipfile.ZIP_DEFLATED) as archive:
        for path in sorted((ROOT / "qbm").rglob("*.py")):
            archive.write(path, path.relative_to(ROOT).as_posix())
        archive.write(SITE / "qbm_web.py", "qbm_web.py")

    (OUT / "data").mkdir(exist_ok=True)
    published = {"headtohead": headtohead(), "constrained_mixer": constrained_mixer(),
                 "penalty_landscape": penalty_landscape(), "flagship_curve": flagship_curve()}
    (OUT / "data" / "published.json").write_text(json.dumps(published, indent=1))
    print(f"built {OUT.relative_to(ROOT)}/ ({sum(1 for _ in OUT.rglob('*') if _.is_file())} files)")


if __name__ == "__main__":
    main()
