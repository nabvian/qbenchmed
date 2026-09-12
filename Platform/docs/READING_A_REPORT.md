# Reading a Q-BenchMed Report

This guide explains the three report areas that are easiest to confuse: the
complete input/outcome inventory, the `K` panel results, and the wiring or
reasoning-layer diagnostics.

## 1. Complete inputs and outcomes

The approved `qbm.profile` is the optimizer's complete problem definition. The
browser presents every record in three drill-downs:

1. **Inputs** — ID, label, cost, tags, wiring status, and linked outcomes.
2. **Outcomes** — ID, label, weight, tags, reachability status, and supporting
   inputs.
3. **Relationships** — every approved `input_id -> outcome_id` incidence.

The reusable profile download contains the same source-of-truth arrays as
`inputs`, `outcomes`, and `relationships`. The full report also retains the
projection evidence, defaulted fields, assumptions, and diagnostics used to
create that profile.

## 2. What K=5 and K=10 mean

`K` is the maximum number of inputs the solver is allowed to select. It is not
the number of outcomes requested.

For example, a K=10 result can legitimately contain seven selected inputs. That
means the selected seven already cover every outcome that the solver can add
from the approved relationships; adding three inert or redundant inputs would
increase cost without increasing coverage.

Open a K result to inspect:

- selected inputs;
- covered and uncovered outcomes;
- selected count and total cost;
- covered weight, total outcome weight, and coverage fraction;
- solver, seed, iterations, and candidates evaluated;
- constraint violations and whether the profile constraints passed; and
- limitations and whether optimality was proven.

The approved classical-optimization download contains every measured K result
in `coverage_at_k`. A result from `greedy`, `simulated_annealing`, or
`tabu_search` is a reproducible baseline, not a proof that no better panel
exists. Only a completed bounded `exact` result can prove optimality.

## 3. “Unwired” has a precise boundary

An input is **inert in the approved qbm.profile** when no approved relationship
starts from that input. An outcome is **unreachable in the approved
qbm.profile** when no approved relationship ends at that outcome.

This statement is intentionally narrower than “the source project has no
logic.” A missing profile relationship can have several evidence-level causes:

- the relationship genuinely is absent;
- the built-in profiler did not recognize the project's custom rule shape;
- a rule was ignored because no supported condition/input reference was found;
- the relevant file format was unsupported or a check was skipped;
- the relationship exists only through generated code, dynamic dispatch, API
  behavior, model execution, macros, or runtime state; or
- the project requires a declarative adapter or explicit profile exporter.

The wiring diagnostic therefore records both the expanded profile graph and
the interpretation context:

- projection method and confidence;
- source-linked projection diagnostics and assumptions;
- parser/check limitations;
- whether an eligible declarative adapter was selected; and
- why the adapter-specific semantic graph, semantic audit, or projection
  catalog was skipped.

## 4. How to improve an incomplete projection

Do not add relationships merely to make the coverage percentage higher. First
confirm the biomedical meaning in the source project, then choose one of these
paths:

1. Export an explicit, reviewed `qbm.profile.json` from the project. This is the
   most reliable and portable option.
2. Install a bounded declarative adapter for a recurring project family whose
   JSON/YAML layout needs interpretation.
3. Correct malformed or unsupported structured evidence and run a new immutable
   audit.

After the corrected profile is approved, Q-BenchMed recomputes reachability,
K panels, minimum panels, solver comparisons, QUBO/Ising structure, and the
version diff against the earlier approved profile with the same `profile_id`.

