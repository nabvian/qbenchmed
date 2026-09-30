# Using Q-BenchMed

Two things live here. The **Python package** is the reference implementation and
the fastest way to see the quantum result. The **Rust Platform** is what you
point at a project of your own.

Start with whichever matches what you want. They do not depend on each other.

## 1. What you need

For the Python package: Python 3.11 or newer.

For the Platform: Rust 1.85 or newer, from [rustup.rs](https://rustup.rs). The
pinned toolchain is 1.95.

Nothing needs network access at run time. Nothing needs a quantum provider
account, and there is nowhere to put one.

## 2. The quantum result

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"

python experiments/run_headtohead.py
```

This is the experiment the project exists for. It builds one QUBO per instance
and hands it to exhaustive search, tabu, simulated annealing and QAOA at depths
one to three, then scores every answer against the certified optimum of that
same QUBO. Takes about seven and a half minutes.

It runs on generated instances of eight to sixteen inputs, because that is what
a state-vector simulator can hold. No private data is involved and the result is
fully reproducible from this repository.

For a faster look, the result tables already ship:

```bash
python experiments/build_report.py
cat reproducibility/REPORT.md
```

## 3. Try it in the browser

`notebooks/qbenchmed_colab.ipynb` runs the same comparison in Google Colab with
no local install. Open it from GitHub with the Colab badge in the README.

## 4. Take a problem or a circuit somewhere else

`qbm/interop.py` exports without requiring anything from this package on the
other side.

```python
from qbm import instances, qubo, interop

inst = instances.generate("degree_capped", 12, 16, seed=0)
q = qubo.build_max_coverage(inst.A, inst.w, inst.c, 4, encoding="pairwise")

open("problem.lp", "w").write(interop.qubo_to_lp(q))
open("penalty.qasm", "w").write(
    interop.to_qasm2(q.n_vars, interop.penalty_qaoa_gates(q, [0.4], [0.9])))
open("constrained.qasm", "w").write(
    interop.to_qasm2(12, interop.constrained_qaoa_gates(inst.A, inst.w, 4, [0.4], [0.9])))
```

The `.lp` file is CPLEX LP, which qiskit-optimization's `QuadraticProgram`,
CPLEX, Gurobi, SCIP and HiGHS all read. The `.qasm` files are OpenQASM 2.0 built
only from `h`, `x`, `rx`, `ry`, `rz` and `cx`, with a measurement on every qubit,
so they load into any OpenQASM 2.0 toolchain as they are.

Replace the angles with ones you have optimised. `ConstrainedQaoa.run` returns
its optimised angles in `parameters`.

The constrained circuit exports only when every outcome is reached by at most
two biomarkers. A rule needing three or more together becomes a multi-body phase
that the exporter does not emit, and it refuses rather than approximating.

## 5. Analyse a profile from the command line

```bash
cd Platform
cargo build --release -p qbm-cli
```

Check what a profile contains:

```bash
./target/release/qbm bench check \
    ../benchmarks/synthetic/conjunctive-40x50.qbm.profile.json
```

You get the structure: how many inputs and outcomes, how many rule arms, how
many of those arms need several inputs together, which inputs no rule mentions,
and which outcomes no selection can reach.

Solve it:

```bash
./target/release/qbm bench analyze \
    ../benchmarks/synthetic/conjunctive-40x50.qbm.profile.json
```

Coverage at each panel size, the smallest panel reaching 80% and 90%, and a
comparison of every solver at one fixed ceiling. On that instance everything
comes back proven in well under a second.

Other subcommands: `solve` runs one solver with your own ceiling or coverage
floor, `qubo` builds and validates the QUBO and Ising forms, and `compare`
diffs two revisions of the same profile.

## 6. Run it on a project of your own

```bash
./target/release/qbm --data-dir .qbenchmed \
    run express --source /path/to/your/project
```

It reads the project, derives a profile if it can, runs the classical analysis
and the QUBO validation, and prints a full report as JSON.

Real projects carry datasets and model weights next to their knowledge files,
and intake counts cumulative bytes, so exclude the bulk:

```bash
./target/release/qbm --data-dir .qbenchmed run express \
    --source /path/to/project \
    --exclude-name .venv --exclude-name models --exclude-name data
```

Exclusions are recorded in the inventory, so the report always states what was
left out.

If it reports that no profile could be built, read
[docs/PREPARING-A-PACK.md](docs/PREPARING-A-PACK.md). The usual cause is that
the project never declares, in a machine-readable field, what domain it belongs
to — and that is two lines to fix.

## 7. The browser interface

```bash
./target/release/qbm --data-dir .qbenchmed serve
```

Opens on `127.0.0.1:8787`. Choose a folder, an archive or a public GitHub
repository, then approve each of fourteen stages against its exact content hash.

Express mode does the same fourteen stages without stopping. The outputs and
hashes are identical; what differs is who accepted them. An express approval is
stamped `auto_accepted_by_policy` rather than a person's name, and the report
says so, so a reviewed result and an unattended one can always be told apart.

The service listens on loopback only. Uploaded project code is read as data and
never executed.

## 8. Reading the output

Three fields decide how much weight an answer carries.

`optimality_proven` is the important one. True means the search closed and the
answer is the best that exists. False means it is a baseline, and no amount of
good-looking coverage changes that.

`coverage_fraction` is weighted by outcome importance, not a count of outcomes.
The covered-outcome count is reported separately.

`K` is a ceiling, not a request. `K=10` means "at most ten inputs" and the panel
may come back smaller when nothing further improves the score.

Two labels describe wiring. **Inert** means no relationship mentions that input.
**Veto-only** means every mention of it blocks an outcome or supplies context,
so selecting it can never grant coverage. **Unreachable** means no selection of
any size covers that outcome — which under conjunctive rules can happen even
when relationships mention it.

None of those are claims about your project beyond the profile it produced.

## 9. Reproducing the published numbers

See [docs/TESTING.md](docs/TESTING.md). It covers both test suites, which files
are protecting what, the measured runtime of every experiment, and what is not
covered.

## 10. If something goes wrong

**"no conservative biomedical profile projection is available"** — the project
declares no domain. See section 6 above.

**"graph exceeds configured node limit"** — the project is too large to map.
Scope the intake with `--exclude-name`.

**"N unsafe source entries stopped this express run"** — intake refused
something. Run in governed mode to see the inventory and decide.

**The certified solver returns `optimality_proven: false`** — the search hit its
node budget. The answer stands as a baseline; the reported gap tells you how
much room was left.

## Feedback

Issues and questions: https://github.com/nabvian/qbenchmed/issues
