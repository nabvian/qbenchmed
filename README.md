# Q-BenchMed

Does quantum optimisation help on a real biomedical decision problem?

Q-BenchMed answers that for one concrete problem: given a set of biomarkers you
could acquire and a set of clinical outcomes a rule engine can reach, which
biomarkers cover the most outcomes? Classical solvers and QAOA receive the
identical QUBO, so any difference between them is the optimiser rather than the
problem.

The benchmark instance is not synthetic. `QBMED-HEME-001` is 66 biomarkers and
88 outcomes, taken from an audit export of a working pathology engine's trigger
matrix. 86 of its 116 rule arms fire only when several biomarkers are present
together, which is what makes it worth solving: coverage is not submodular and
greedy selection stalls where no single biomarker shows a gain.

## What it found

On that instance, with every solver given the same QUBO:

| method | formulation | reaches the certified optimum |
|---|---|---|
| exhaustive | QUBO | 100% |
| tabu | QUBO | 100% |
| QAOA p=2 | QUBO | 92% |
| annealing | QUBO | 88% |
| greedy | native | 80% |
| QAOA p=3 | QUBO | 72% |
| QAOA p=1 | QUBO | 60% |

QAOA at depth two is comparable to the classical heuristics, above annealing
and greedy and below tabu. With 25 instances per configuration those rates are
not separated by the data, so the honest reading is comparable, not better.

An earlier revision of this repository reported that classical heuristics beat
QAOA at every depth. That conclusion was an artefact of the parameter search,
not of the ansatz: gamma was drawn from half its period, and the penalised
spectrum puts hundreds of local minima inside one period, so three random
restarts of a local optimiser was a lottery. Section 7 of the report records
what changed and why.

The result that survived is about the encoding. The budget constraint is
expressed as a penalty, and those penalty terms run 85 to 374 times larger than
the coverage signal, leaving under 1% of the energy spectrum holding anything
useful. Replacing the penalty with a constraint-preserving mixer is the next
experiment, and it is the one the measurements point at.

No quantum advantage is claimed. Nothing here extrapolates beyond 16 inputs,
which is where state-vector simulation stops.

## Two implementations

| | `qbm/` (Python) | `Platform/` (Rust) |
|---|---|---|
| Role | reference implementation; establishes the mathematics | the product you run on your own projects |
| Instance | `QBMED-HEME-001` | anything you point it at |
| Quantum | QAOA on a state-vector simulator, with noise | QUBO/Ising formulation, export, local Ising solver |
| Entry point | `experiments/*.py` | `qbm serve`, `qbm run express`, `qbm bench` |

They share a problem definition and nothing else. The Python package does not
import the Platform, and the Platform never executes the Python package. Their
certified solvers were written independently and agree to the digit on the
reference instance, which is the strongest check either of them gets.

## Running the Platform

```bash
cd Platform
cargo build --release -p qbm-cli

# analyse a profile directly
./target/release/qbm bench analyze \
    crates/qbm-benchmark/tests/fixtures/qbmed-heme-001.qbm.profile.json

# run a whole project unattended
./target/release/qbm --data-dir .qbenchmed run express --source /path/to/project

# or the browser interface, with an approval at each of fourteen checkpoints
./target/release/qbm --data-dir .qbenchmed serve
```

Express records the same stage outputs and content hashes as a governed run.
What differs is who accepted them: approvals are stamped
`auto_accepted_by_policy` rather than a person, and the report says so.

The service listens on `127.0.0.1` only. Uploaded project code is read as data
and never executed.

## Running the Python package

Python 3.11 or later. Runtime dependencies are numpy, scipy and pandas. Qiskit
is a validation reference used by 40 cross-check tests and lives in the `dev`
extra; the library never imports it.

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
pytest tests/ -q
```

```
qbm/
  instances.py      instance generators, six structural regimes
  profile.py        domain profiles: typed edges and path groups
  domains/heme.py   the 66-biomarker haematology adapter
  qubo.py           QUBO construction, three encodings, Ising conversion
  classical.py      exhaustive, ILP via HiGHS, greedy, annealing, tabu
  qaoa.py           QAOA ansatz, grid-seeded parameter search
  simulator.py      state-vector simulator with gate resource accounting
  noise.py          trajectory-based noise channels
  runner.py         one code path that prepares every solver's problem
  export.py         projection onto the Platform's profile schema
```

## Documentation

[docs/TESTING.md](docs/TESTING.md) covers running both suites, reproducing the
published results, and what the tests do not cover.

[docs/PREPARING-A-PACK.md](docs/PREPARING-A-PACK.md) explains how to prepare
your own project so the Platform can read it, including the declared-domain
requirement that most projects miss on the first attempt.

[docs/formulation.md](docs/formulation.md) gives the mathematical formulation.

[reproducibility/REPORT.md](reproducibility/REPORT.md) is the full result
report. Every number in it is read from a result table rather than transcribed,
and since the last revision its conclusions are derived from those tables too.

[Platform/README.md](Platform/README.md) documents the fourteen stages, the
profile contract, and the safety boundaries.

## Scope, and what this is not

`QBMED-HEME-001` records which biomarkers each rule reads. It contains no
patient data and no clinical knowledge.

A covering panel is a solution to a coverage objective over a rule graph. It
does not follow that any biomarker is clinically unnecessary, that a smaller
panel is safe, or that the outcome set is clinically validated because it can
be optimised. Q-BenchMed is not a clinical validator, a diagnostic device, or a
regulatory assessment.

The quantum side is simulated. The executor interface is provider-neutral and
its only implementation is a local classical Ising solver, which reports itself
as classical. No result here was produced on quantum hardware.

## Licence

GNU Affero General Public License, version 3 or later. See [LICENSE](LICENSE)
and [NOTICE](NOTICE).

Section 13 matters here because the Platform serves a browser interface over
HTTP. If you run a modified version and let other people use it over a network,
you have to offer them the source of your modified version. Running it on your
own machine, which is the normal case, triggers nothing.
