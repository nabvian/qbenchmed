# Benchmark invariants

Seven rules this project does not break. They exist because a benchmark that
bends them stops measuring anything, and the bending is usually invisible in
the output.

## 1. Every solver sees the identical problem

A classical solver and QAOA receive the same QUBO, built once, with the same
penalty coefficients. Any difference in their answers is the optimiser, not the
problem.

Where a solver works on the native formulation instead of the QUBO — greedy
does — it is reported on its own row and never averaged across the boundary.
The two comparisons answer different questions and the report keeps them apart.

## 2. Optimality is claimed only when it is proven

`optimality_proven` is true for exhaustive enumeration that completed inside
its bound, and for the certified branch-and-bound when its search closed. It is
false for greedy, annealing and tabu, always.

When the certified search exhausts its node budget it returns the best answer it
found, marks it unproven, and reports the gap that remained. It never promotes a
good answer to a proven one.

## 3. The engine is domain-blind

`qbm-benchmark` optimises inputs and outcomes. It has no notion of a biomarker,
a disease or a patient. A profile declares what its inputs mean through
`input_semantics`, and that string is documentation, never logic.

Nothing infers biomedical meaning from a symbol name. A Rust function called
`EGFR_positive` is a function, not a biomarker.

## 4. Coverage is conjunctive where the rules are

A rule's conditions hold together, so its inputs are needed together. Arms carry
that through the schema, the solvers, the QUBO and the metrics.

Flattening an arm into independent edges is not a simplification, it is a
different and easier problem: it makes coverage submodular again, removes
outcomes that are genuinely unreachable, and lets greedy look optimal where it
is not.

## 5. Missing evidence is reported, not invented

When a project supplies nothing a profile can be built from, the profile,
comparison, optimisation and QUBO stages are recorded as skipped with a reason.
They are never filled with a guess.

Inert inputs, veto-only inputs and unreachable outcomes describe the approved
profile and nothing beyond it. They are not claims about the project's source
code.

## 6. No quantum advantage is claimed

The quantum side runs on a state-vector simulator, at sizes a simulator can
reach. Nothing here extrapolates past sixteen inputs.

Wall-clock time is never used to rank a simulated quantum circuit against a
classical solver. Gate counts are charged against the standard hardware
decomposition, not against the simulator's shortcuts, because reporting
simulator convenience as circuit cost would flatter QAOA in exactly the
dimension the benchmark exists to measure.

The executor interface is provider-neutral. Its only implementation is a local
classical Ising solver, which reports itself as classical in its own readiness
output.

## 7. Results are generated, and conclusions follow them

Every number in the report is read from a result table. Since the last revision
the report's conclusions are derived from those tables too, because a
regenerated report that keeps a sentence its own numbers have overturned is
worse than no report.

When a result changes, the text changes with it. The report carries a record of
the three defects found so far and what each one changed, including one that
reversed a published conclusion.

## What follows from all this

The project reports a negative or inconclusive result wherever that is what the
measurement says. It has done so twice: once when QAOA lost, and once when
fixing the parameter search showed the loss had been partly the author's own
optimiser rather than the ansatz.

Neither outcome is a failure of the benchmark. Publishing the second one was the
point of building it carefully enough to notice.
