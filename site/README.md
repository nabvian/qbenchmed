# Browser playground (GitHub Pages)

**Live: https://nabvian.github.io/qbenchmed/**

A static page that runs the unmodified `qbm` package in the browser with
[Pyodide](https://pyodide.org) (numpy, scipy and pyyaml compiled to
WebAssembly). Nothing to install and no server.

- **What it found**: the published head-to-head, constrained-mixer and
  penalty-landscape results, summarised from `results/*.csv` at build time.
- **Every solver, one QUBO**: generate an instance and run exhaustive, greedy,
  annealing, tabu and QAOA (p = 1–3) on the identical QUBO, plus the
  penalty-free constrained mixer. Up to 12 inputs.
- **Where greedy stalls**: re-solve `QBMED-HEME-001` with the certified ILP and
  greedy, and check the result against the published curve.

| file | role |
|---|---|
| `qbm_web.py` | the bridge: picks what to run, returns JSON; mirrors the Colab notebook |
| `static/` | page, styles, charts (`app.js`) and the Pyodide worker (`worker.js`) |
| `build.py` | writes `_site/`: static files, `qbm.zip` (package + bridge), `data/published.json` |
| `dev_server.py` | local test server: serves `_site/` and runs the bridge with CPython |

```bash
python3 site/build.py                 # standard library only
python3 site/dev_server.py            # needs numpy, scipy, pyyaml
# open http://localhost:8000/?backend=local   (CPython backend)
# open http://localhost:8000/                 (Pyodide, as on GitHub Pages)
```

`.github/workflows/pages.yml` rebuilds and publishes to the `gh-pages` branch
on every push to `main` that touches `qbm/`, `results/` or `site/`.
GitHub Pages serves that branch (Settings → Pages → Deploy from a branch →
`gh-pages`, `/ (root)`).

The playground offers the `degree_capped` and `low_overlap` regimes only: they
admit the pairwise QUBO (at most 15 qubits at 12 inputs). The other regimes need
the slack encoding at 23–79 qubits, beyond a state-vector simulator.
