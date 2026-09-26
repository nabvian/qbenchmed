# How to test Q-BenchMed

There are two independent implementations in this repository and they are
tested separately. The Rust Platform is the product. The Python package is the
reference implementation that the Platform's certified solver is checked
against.

Run both. They check different things, and the interesting failures are the
ones where they disagree.

## Rust Platform

```bash
cd Platform
cargo test --workspace --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo fmt --all -- --check
```

147 tests. The suite takes about twenty seconds; most of that is the certified
solver working on the real 66-biomarker instance.

`qbm-benchmark` is compiled with optimisation even in the test profile. The
certified search explores millions of nodes and an unoptimised build turns a
twenty-second suite into a ten-minute one. The search is integer-deterministic,
so the optimisation level changes speed and nothing else.

Three test files carry most of the weight:

`tests/ilp_certified.rs` generates random profiles with conjunctive arms,
exclusions and contexts, then solves each one at every panel size both by
certified branch-and-bound and by enumerating every subset. They have to agree.
This is what stops a bad bound from silently discarding the optimum while still
reporting `optimality_proven = true`.

`tests/reference_instance.rs` runs the real QBMED-HEME-001 instance and checks
the answers against numbers produced independently by the Python package
through SciPy/HiGHS. The two solvers share no code and no method. If a change
to the bound breaks one of them, this file fails rather than a result quietly
changing.

`tests/qubo_conjunctive.rs` enumerates whole QUBO spectra, decodes the ground
state back to a panel, and requires it to match what the certified solver
proves natively. A QUBO that encodes the wrong problem is internally consistent
and passes every other check, so this one compares it against something
outside itself.

## Python reference implementation

```bash
python3 -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
pytest tests/ -q
```

354 tests, about twenty seconds. 40 of them cross-check the state-vector
simulator against Qiskit and skip cleanly if Qiskit is absent, which is why a
run without the `dev` extra reports 314 passed and 1 skipped.

The library never imports Qiskit. It is a validation reference, not a
dependency.

## Reproducing the published results

The result tables in `results/` and the report in `reproducibility/` are
generated, not written by hand.

```bash
python experiments/run_flagship.py              #   2 s
python experiments/run_discriminating_power.py  #  13 s
python experiments/build_report.py              # regenerates reproducibility/
```

Those three are enough to rebuild the report, because the two slower
experiments ship their result CSVs:

```bash
python experiments/run_headtohead.py   # 7.6 min, state-vector simulation
python experiments/run_noise.py        # 1.1 min, 279 points x 400 trajectories
```

Each script records its own measured runtime and environment in
`results/*_env.json`.

If you re-run the head-to-head or the noise sweep, regenerate the report
afterwards. The report reads its numbers from the CSVs, and since it also
derives its conclusions from them, a stale CSV produces a report that argues
against its own tables.

## Checking the Platform against a profile by hand

The CLI runs the benchmark half without a browser or a governed run:

```bash
cd Platform
cargo build --release -p qbm-cli

./target/release/qbm bench check \
    crates/qbm-benchmark/tests/fixtures/qbmed-heme-001.qbm.profile.json

./target/release/qbm bench analyze \
    crates/qbm-benchmark/tests/fixtures/qbmed-heme-001.qbm.profile.json
```

`check` validates a profile and reports its structure: how many biomarkers and
outcomes, how many rule arms, how many of those arms need a combination of
biomarkers rather than any one of them, and which outcomes no selection can
reach.

`analyze` runs the whole classical analysis. On the reference instance it takes
about thirteen seconds and every answer comes back proven.

For the full governed workflow, with an approval at each of the fourteen
checkpoints:

```bash
./target/release/qbm --data-dir .qbenchmed serve
```

To run the same stages unattended:

```bash
./target/release/qbm --data-dir .qbenchmed run express --source /path/to/project
```

Express records the same stage outputs and the same content hashes as a
governed run. What differs is who accepted them: approvals are stamped
`auto_accepted_by_policy` instead of a person, and the report says so.

## What the tests do not cover

Repeatable end-to-end fixtures across operating systems. Everything here is
developed and run on macOS.

Fuzz, property and security testing beyond the generated-profile tests in
`ilp_certified.rs`.

Accessibility automation for the browser interface.

Any quantum hardware. The executor interface is provider-neutral and the only
implementation is a local classical Ising solver, which says so in its own
readiness report.
