# Q-BenchMed

[![License: AGPL v3](https://img.shields.io/badge/License-AGPL%20v3-blue.svg)](LICENSE)
[![Open in Colab](https://colab.research.google.com/assets/colab-badge.svg)](https://colab.research.google.com/github/nabvian/qbenchmed/blob/main/notebooks/qbenchmed_colab.ipynb)

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
useful.

That made a prediction, and it has now been tested. A constrained ansatz that
never leaves the set of valid panels (a Dicke-state start and an XY ring mixer,
with no penalty at all) was run on the same instances:

| depth | penalty QAOA vs a random valid panel | constrained QAOA vs a random valid panel |
|---|---|---|
| 1 | 0.02× | 5.0× |
| 2 | 0.26× | 9.1× |
| 3 | 0.05× | 13.3× |

The penalty version almost always does worse than guessing; the constrained
one beats guessing on every instance, with its largest margin at the largest
size tested. It also runs on slices
of the real instance using 10 to 16 qubits, where the penalty encoding needs 85
to 120. Both ansatzes were reproduced independently on Qiskit, 18 of 18 runs in
agreement.

No quantum advantage is claimed. Everything is noiseless simulation, nothing
extrapolates beyond 16 inputs, and at the sizes that can be simulated greedy
still finds the optimum.

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

Or run the comparison in the browser with no local install:
[notebooks/qbenchmed_colab.ipynb](notebooks/qbenchmed_colab.ipynb), about four
minutes on a free CPU runtime.

```
qbm/
  instances.py      instance generators, six structural regimes
  profile.py        domain profiles: typed edges and path groups
  domains/heme.py   the 66-biomarker haematology adapter
  qubo.py           QUBO construction, three encodings, Ising conversion
  classical.py      exhaustive, ILP via HiGHS, greedy, annealing, tabu
  qaoa.py           QAOA ansatz, grid-seeded parameter search
  constrained.py    constrained-mixer QAOA: Dicke state, XY ring, no penalty
  interop.py        CPLEX LP and OpenQASM 2.0 export, Dicke-state circuit
  simulator.py      state-vector simulator with gate resource accounting
  noise.py          trajectory-based noise channels
  runner.py         one code path that prepares every solver's problem
  export.py         projection onto the Platform's profile schema
```

## Documentation

| | |
|---|---|
| [USAGE.md](USAGE.md) | running both halves, end to end |
| [RESULTS.md](RESULTS.md) | everything measured so far, in one page |
| [BENCHMARK_INVARIANTS.md](BENCHMARK_INVARIANTS.md) | the seven rules the benchmark does not break |
| [reproducibility/REPORT.md](reproducibility/REPORT.md) | the full result report, regenerated from the tables |
| [docs/TESTING.md](docs/TESTING.md) | both suites, reproducing the numbers, what is not covered |
| [docs/PREPARING-A-PACK.md](docs/PREPARING-A-PACK.md) | preparing your own project so the Platform can read it |
| [docs/formulation.md](docs/formulation.md) | the mathematics |
| [Platform/README.md](Platform/README.md) | the fourteen stages, the profile contract, the safety boundaries |

Every number in the report is read from a result table rather than
transcribed, and since the last revision its conclusions are derived from those
tables too. The raw tables are in [results/](results/), one CSV per experiment
with the environment and measured runtime recorded alongside.

## Status, honestly

The classical side is finished and cross-validated: two certified solvers
written independently, sharing no code, agreeing to the digit on the reference
instance.

The quantum side is a simulation, at sizes a state-vector simulator can hold.
It has produced two results: the penalty encoding wastes the spectrum, and a
constrained mixer that removes the penalty beats random guessing where the
penalty version cannot. Both are noiseless. Nothing has run on hardware, and
the constrained mixer's advantage has not been tested under noise, where its
state-preparation cost counts against it.

The circuits export as OpenQASM 2.0 and the problem as a CPLEX LP file, so
either can be taken to another toolchain without this package.

Four defects have been found and recorded so far. One invalidated a whole
noise sweep; one reversed the headline conclusion; one was the first of those
recurring in new code, caught by the Qiskit cross-check before it reached a
result. Each is written up in section 7 of the report, with what it changed, and
the invalidated table is kept rather than deleted. Expect more.

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

## Collaboration

Koushik Das — engikd1993@gmail.com

Bugs, questions and disagreements with the conclusions are all welcome as
issues. A disagreement backed by a run is the most useful kind: the result
tables and the environment fingerprints are in the repository precisely so that
one can be checked against another.

Three things would particularly benefit from another pair of hands:

- **The constrained-mixer experiment.** An XY mixer over a fixed Hamming-weight
  subspace removes the penalty term the measurements point at. The
  infrastructure to run and score it is already here; the mixer is not.
- **Hardware.** Everything is simulated. The executor interface is
  provider-neutral and unused, and the noise sweep says two-qubit error is what
  will decide a real run.
- **More instances.** The benchmark is built around one real conjunctive
  instance. A second one, from a different domain, would say a great deal about
  which of these results are about the problem and which are about this problem.

If you have a biomedical project you want analysed rather than a quantum method
you want tested, [docs/PREPARING-A-PACK.md](docs/PREPARING-A-PACK.md) is the
place to start.
