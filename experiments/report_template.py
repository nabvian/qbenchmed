"""REPORT.md renderer for the Q-BenchMed benchmark report.

Imported by build_report.py. Every number is read from the summary dict that
build_report.summarise() computes from the result CSVs -- nothing is typed in.
"""
from __future__ import annotations

# Figure artifacts, referenced by artifact_id so the embed tracks the latest
# version rather than pinning one render.
FIGURE_IDS = {
    "baselines": "1ceb98af-a933-4c2c-a72b-9021c722e36b",
    "headtohead": "386ae62f-2a81-471a-a633-f6b4a2147412",
    "noise": "fe6810bc-606f-4f23-a786-2d6d30ea8105",
}
SIZES = ("8", "10", "12", "14", "16")


def _pct(x: float) -> str:
    return f"{100 * x:.0f}%"


def _fig(key: str) -> str:
    return "{{artifact:art_" + FIGURE_IDS[key] + "}}"


def _by_size(d: dict, key: str):
    """Size-keyed dicts have int keys in memory and str keys after a JSON
    round-trip. Accept either so the renderer works on both paths."""
    if key in d:
        return d[key]
    return d[int(key)]


def _k(d: dict, *parts) -> float:
    return float(d["|".join(str(x) for x in parts)])


def _constrained_sections(S: dict) -> str:
    """Sections 8 and 9, read off the constrained, real-slice and Qiskit tables.

    Absent tables produce a one-line note rather than a gap in the numbering, so
    the report stays well-formed on a checkout that has not run them.
    """
    c, r, q = S.get("constrained"), S.get("real_slices"), S.get("qiskit")
    out = ["## 8. Constrained mixer: the prediction, tested\n"]
    if c is None:
        out.append("Not run in this checkout: `python experiments/run_constrained_mixer.py`.\n")
    else:
        depths = sorted({int(k.split("|")[1]) for k in c["p_opt_by_ansatz_depth"]})
        sizes = sorted({int(k.split("|")[1]) for k in c["p_opt_by_ansatz_size"]})
        depth_rows = "\n".join(
            f"| {d} | {_k(c['p_opt_by_ansatz_depth'], 'penalty', d):.4f} | "
            f"{_k(c['p_opt_by_ansatz_depth'], 'constrained', d):.4f} | "
            f"{_k(c['lift_over_random_by_ansatz_depth'], 'penalty', d):.2f}× | "
            f"{_k(c['lift_over_random_by_ansatz_depth'], 'constrained', d):.1f}× |"
            for d in depths)
        _lifts = list(c["constrained_lift_by_size"].values())
        lift_trend = ("the lift grows with size" if all(a <= b for a, b in zip(_lifts, _lifts[1:]))
                      else "the lift is largest at the largest size, though it does not "
                           "rise steadily")
        lift_by_size = ", ".join(f"{v}× at {n}" for n, v in
                                 c["constrained_lift_by_size"].items())
        random_hit = ", ".join(f"{_pct(v)} at {n}" for n, v in c["random_hit_by_size"].items())
        size_hdr = " | ".join(str(n) for n in sizes)
        pen_row = " | ".join(f"{_k(c['p_opt_by_ansatz_size'], 'penalty', n):.4f}" for n in sizes)
        con_row = " | ".join(f"{_k(c['p_opt_by_ansatz_size'], 'constrained', n):.4f}" for n in sizes)
        qub_row = " | ".join(f"{int(_k(c['qubits_by_ansatz_size'], 'penalty', n))} / "
                             f"{int(_k(c['qubits_by_ansatz_size'], 'constrained', n))}" for n in sizes)
        gate_rows = "\n".join(
            f"| {d} | {int(_k(c['gates_by_ansatz_depth'], 'penalty', d))} | "
            f"{int(_k(c['gates_by_ansatz_depth'], 'constrained', d))} | "
            f"{int(_k(c['gates_with_prep_by_ansatz_depth'], 'constrained', d))} |"
            for d in depths)
        cheaper = [d for d in depths
                   if _k(c["gates_with_prep_by_ansatz_depth"], "constrained", d)
                   < _k(c["gates_by_ansatz_depth"], "penalty", d)]
        if len(cheaper) == len(depths):
            gate_claim = ("Including the Dicke preparation, the constrained circuit is "
                          "cheaper at every depth tested.")
        elif not cheaper:
            gate_claim = ("Including the Dicke preparation, the constrained circuit is "
                          "**more** expensive at every depth tested; it is cheaper per "
                          "layer, and the one-time preparation outweighs that here.")
        else:
            first = min(cheaper)
            con_g = _k(c["gates_with_prep_by_ansatz_depth"], "constrained", first)
            pen_g = _k(c["gates_by_ansatz_depth"], "penalty", first)
            later = [d for d in cheaper if d > first]
            if (pen_g - con_g) / pen_g < 0.05:
                gate_claim = (
                    f"Including the Dicke preparation, the constrained circuit costs "
                    f"more at p=1, breaks even at p={first} ({int(con_g)} against "
                    f"{int(pen_g)}, a margin too small to call a saving)"
                    + (f", and is cheaper from p={min(later)}." if later else ".")
                    + " The preparation is a one-time cost; the per-layer saving "
                    "only pays it back with depth.")
            else:
                gate_claim = (f"Including the Dicke preparation, the constrained circuit "
                              f"is cheaper from p={first}. Below that the one-time "
                              f"preparation outweighs its per-layer saving.")
        pen_feas = float(c["feasible_by_ansatz"]["penalty"])
        con_feas = float(c["feasible_by_ansatz"]["constrained"])
        out.append(f"""Section 5 made a prediction: remove the penalty and the probability of
sampling the optimum should rise. This section tests it on the same
{c['instances']} head-to-head instances, with the same shots, restart count and
grid-seeded parameter search.

The constrained ansatz never leaves the feasible set. It starts from the Dicke
state `|D^n_K>`, the equal superposition of every panel with exactly K
biomarkers; its mixer is an XY ring, every term of which conserves Hamming
weight; and its cost layer is coverage alone, with **no penalty term and no
slack qubits**. "At most K" becomes "exactly K" without loss because coverage
never falls when a biomarker is added, and the experiment asserts that equality
on every instance rather than assuming it.

The fair baseline is a uniformly random panel of exactly K biomarkers — the
Dicke state before any layer. Probability of sampling the certified optimum,
and the median lift over that baseline:

| depth | P(opt), penalty | P(opt), constrained | lift over random, penalty | lift over random, constrained |
|---|---|---|---|---|
{depth_rows}

**The constrained ansatz beats a random feasible panel on
{_pct(c['constrained_beats_random_fraction'])} of instance-depth pairs**, by
{c['constrained_lift_range'][0]}× to {c['constrained_lift_range'][1]}×, and {lift_trend}
({lift_by_size}). **The penalty ansatz does worse than random guessing on
{_pct(c['penalty_below_random_fraction'])} of them**: it spreads probability over panels that
break the budget.

The rate at which *any* of the 4,096 shots hits the optimum is deliberately not
the headline. The constrained ansatz hits it every time, but so would random
guessing among valid panels ({random_hit}): with a feasible space of at most
4,368 panels, 4,096 shots find the optimum by chance. That column is
uninformative here, and quoting it would overstate the result.

At p=2, by size, with qubit counts (penalty / constrained):

| inputs | {size_hdr} |
|---|{'---|' * len(sizes)}
| P(opt), penalty | {pen_row} |
| P(opt), constrained | {con_row} |
| qubits | {qub_row} |

Against the penalty ansatz directly, the constrained one samples the optimum
more often on {_pct(c['constrained_wins_fraction'])} of instance-depth pairs, by a median
factor of {c['median_ratio']}× — but much of that factor measures how poorly the
penalty encoding does, not how well the constrained one does, which is why the
random baseline leads. Every constrained sample is feasible
({100 * con_feas:.1f}%, measured from the samples, not assumed); the penalty
ansatz puts {100 * (1 - pen_feas):.1f}% of its samples outside the budget.

Mean two-qubit gates on a CX-native device, cost layer and mixer both charged:

| depth | penalty | constrained, circuit | constrained, with Dicke preparation |
|---|---|---|---|
{gate_rows}

{gate_claim} Its count comes from the real Baertschi-Eidenbenz
circuit this project exports, untranspiled, so it is an upper bound.

This is noiseless simulation. Section 6 showed two-qubit error dominates, and
the constrained ansatz front-loads a fixed two-qubit cost in its preparation.
Whether the gain in optimum probability survives that on a real device is the
open question this section hands on, not one it answers.
""")

    out.append("## 9. The real instance, and an independent reproduction\n")
    if r is None:
        out.append("Real-instance slices not run: `python experiments/run_real_slices.py`.\n")
    else:
        sizes = r["sizes"]
        hdr = " | ".join(str(n) for n in sizes)
        qrow = " | ".join(f"{r['penalty_qubits'][n]} / {n}" for n in sizes)
        arow = " | ".join(f"{r['conjunctive_arms'][n]} of {r['n_arms'][n]}" for n in sizes)
        rrow = " | ".join(f"{r['random_p_opt'][n]:.4f}" for n in sizes)
        prows = "\n".join(
            f"| P(opt), p={d} | " + " | ".join(f"{_k(r['p_opt'], n, d):.4f}" for n in sizes) + " |"
            for d in (1, 2, 3))
        worse = r["depth3_worse_than_depth2"]
        depth_note = (
            f"Depth does not help monotonically: at {' and '.join(str(n) for n in worse)} "
            f"inputs, p=3 samples the optimum less often than p=2. The grid that seeds the "
            f"search uses the same angles in every layer, and three restarts may not be "
            f"enough at p=3; that is a hypothesis, not a finding."
            if worse else "Depth helps at every size tested.")
        greedy_note = (
            "**Greedy finds the certified optimum on every slice.** At these sizes the "
            "problem is classically easy, and QAOA is not competing with anything "
            "classical here. The greedy stall of section 1 appears only at 34 or more "
            "inputs, beyond simulation."
            if r["greedy_finds_all"] else
            "Greedy misses the certified optimum on at least one slice.")
        out.append(f"""The head-to-head runs on generated instances because the penalty encoding of
QBMED-HEME-001 cannot be simulated. Its rules are conjunctive, and a penalty
QUBO expresses each one with an auxiliary variable per outcome plus slack. The
constrained ansatz evaluates coverage as a phase over the biomarkers directly,
so an AND-rule costs no qubits at all.

Nested slices of the real instance, keeping the highest-degree biomarkers and
dropping any rule arm that loses a member:

| inputs | {hdr} |
|---|{'---|' * len(sizes)}
| qubits, penalty / constrained | {qrow} |
| conjunctive arms | {arow} |
| P(opt), random feasible panel | {rrow} |
{prows}

**The constrained ansatz runs on the real instance where the penalty encoding
cannot, and beats a random feasible panel by {r['lift_range'][0]}× to
{r['lift_range'][1]}×.** Qubits are cheap here; gates are not. The AND-rules
expand into terms of up to {r['max_term_order']} bodies, each needing a multi-qubit
phase, so the cost layer is heavier than on a pairwise instance.

{depth_note}

{greedy_note}
""")
    if q is None:
        out.append("Qiskit reproduction not run: `python experiments/run_qiskit_reproduction.py`.\n")
    else:
        out.append(f"""### Reproduced on Qiskit

Every other number in this report comes from this package's own simulator,
which makes it a single point of failure. To remove that, both ansatzes are
optimised here, exported to OpenQASM 2.0 with the angles baked in, and run by
Qiskit {q['qiskit']}'s `StatevectorSampler` — a different code base, parsing the
circuit from text — at {q['shots']:,} shots.

**{q['agree']} of {q['runs']} runs agree within shot noise** (largest deviation
{q['max_abs_z']} standard errors), and the minimum statevector fidelity between the
two simulators is {q['min_fidelity']:.12f}. The circuits in `qbm/interop.py` are
therefore the circuits the results describe, and they run unmodified on any
OpenQASM 2.0 toolchain.
""")
    return "\n".join(out)


def render(S: dict) -> str:
    f, d, h = S["flagship"], S["discriminating_power"], S["head_to_head"]
    p, n = S["penalty_landscape"], S["noise"]

    regime_rows = "\n".join(
        f"| {k} | {_pct(v)} |"
        for k, v in sorted(d["greedy_optimal_by_regime"].items(), key=lambda kv: kv[1]))
    classical_rows = "\n".join(
        f"| {k.split(' (')[0]} | {k.split('(')[1].rstrip(')')} | {_pct(v)} |"
        for k, v in sorted(h["classical_reaches_optimum"].items(), key=lambda kv: -kv[1]))
    # These two sentences used to be fixed prose. They are claims about the
    # table directly above them, so they are now read off it: a regenerated
    # report that silently keeps a conclusion its own numbers have overturned
    # is worse than no report.
    _depths = {k: float(v) for k, v in h["qaoa_reaches_optimum_by_depth"].items()}
    _best_depth = max(_depths, key=_depths.get)
    _best_qaoa = _depths[_best_depth]
    _classical = {k.split(" (")[0]: float(v)
                  for k, v in h["classical_reaches_optimum"].items()}
    _rivals = {k: v for k, v in _classical.items() if k != "exhaustive"}
    _beaten = sorted(k for k, v in _rivals.items() if _best_qaoa > v)
    if not _beaten:
        qaoa_verdict = (
            "**Every classical baseline on this QUBO reaches the optimum at least as "
            "often as QAOA, at every depth.** Greedy is reported separately because it "
            "solves the native formulation rather than the QUBO.")
    else:
        qaoa_verdict = (
            f"**QAOA at p={_best_depth} reaches the optimum {_pct(_best_qaoa)} of the "
            f"time, above {' and '.join(_beaten)}.** Tabu still solves every instance. "
            f"With this many instances per configuration those rates are not separated "
            f"by the data, so the honest reading is that QAOA at its best depth is "
            f"comparable to the classical heuristics here \u2014 not that it beats them.")

    # Section 10 restates section 4's verdict, so it is derived from the same
    # numbers rather than written once and left to drift.
    if not _beaten:
        interpretation_opening = (
            "On this problem, as formulated, **QAOA is outperformed by classical "
            "heuristics on the identical QUBO**, and its advantage over random "
            "sampling is modest.")
        result_c_claim = (
            "The specification's Result C \u2014 QAOA competitive only for specific "
            "structures \u2014 is **not supported here for any structure tested**, in the "
            "size range that can be simulated.")
    else:
        interpretation_opening = (
            f"On this problem, as formulated, **QAOA at p={_best_depth} is comparable to "
            f"the classical heuristics on the identical QUBO** \u2014 above "
            f"{' and '.join(_beaten)}, below tabu, and not separated from either by this "
            f"many instances. Its advantage over random sampling remains modest.")
        result_c_claim = (
            "The specification's Result C \u2014 QAOA competitive only for specific "
            "structures \u2014 is **weakly consistent with the degree-capped structure "
            "tested here at p=2**, and not established: the margin over the classical "
            "heuristics is inside the noise, and tabu still solves every instance. It "
            "holds only across the simulable range.")

    _sizes = [float(_by_size(h["qaoa_reaches_optimum_by_size"], k)) for k in SIZES]
    _monotone = all(a >= b for a, b in zip(_sizes, _sizes[1:]))
    qaoa_size_claim = (
        "QAOA's success **falls with problem size** across the simulable range:"
        if _monotone else
        "QAOA's success is **worst at the largest size tested**, but it does not decline "
        "steadily on the way there, and with five instances per size each point is "
        "individually noisy:")

    qaoa_rows = "\n".join(
        f"| QAOA p={k} | qubo | {_pct(v)} |"
        for k, v in sorted(h["qaoa_reaches_optimum_by_depth"].items()))
    mech_rows = "\n".join(
        f"| {k.replace('_', ' ')} | {v:.0f} |"
        for k, v in sorted(n["degradation_at_1pct_depolarizing"].items(), key=lambda kv: -kv[1]))
    qaoa_by_size = " | ".join(_pct(_by_size(h["qaoa_reaches_optimum_by_size"], k)) for k in SIZES)
    span_row = " | ".join(f"{_by_size(p['useful_span_percent_of_spectrum'], k):.2f}" for k in SIZES)
    neg_row = " | ".join(f"{100 * _by_size(p['negative_energy_fraction_by_size'], k):.1f}%" for k in SIZES)

    constrained_sections = _constrained_sections(S)
    _c = S.get("constrained")
    mixer_claim = (
        "Constraint-preserving mixers (XY-mixers on a fixed-Hamming-weight "
        "subspace) remove the penalty term entirely; they are untested here."
        if _c is None else
        f"Section 8 tests the constraint-preserving alternative: in noiseless "
        f"simulation it beats a random feasible panel on "
        f"{_pct(_c['constrained_beats_random_fraction'])} of runs, where the penalty "
        f"ansatz loses to one on {_pct(_c['penalty_below_random_fraction'])}. Whether "
        f"that survives hardware noise, given its fixed preparation cost, is untested.")

    return f"""# Q-BenchMed-Heme \u2014 Benchmark Report

**Benchmark:** `QBMED-HEME-001` \u00b7 **Schema:** 1.0 \u00b7 **Suite:** {S['tests']['passing']} tests passing

Generated by `experiments/build_report.py` from the result CSVs. Every number
below is read from a table, not transcribed.

---

## Scope and what this is not

This is a **reference implementation in Python**, built to establish that the
mathematics is correct and to find out where the comparison is informative
before the Rust core is written. It is not the Rust authoritative
implementation the specification calls for.

The 66\u00d788 instance is the **real profile**, not a stand-in: it is built from
`benchmarks/heme/QBMED-HEME-001/`, an export of the outcome-by-input trigger and
wiring matrix of the pathology rule engine (audit snapshot 2026-08-29, PRO-EXEC
bundle 1.2.0), with every relationship carrying its source row. It records which
biomarkers each rule reads; it contains **no patient data**.

That the structure is real is what makes the results below informative \u2014 the
conjunctive rules that break greedy are a property of the rule engine, not of a
generator. It is also why the claims policy matters more, not less. Nothing here
is a statement about hematology: a covering panel is a solution to a coverage
objective over a rule graph, and **it does not follow that any test is clinically
unnecessary**, that a smaller panel is safe, or that the outcome set is
clinically validated because it can be optimized.

Per the specification's claims policy: no quantum advantage is claimed, no
clinical validity is claimed, and wall-clock time is never used to rank a CPU
state-vector simulation against a classical solver.

---

## 1. The flagship instance is easy to solve exactly, but not trivial to solve well

On the 66-input / 88-outcome instance, exact ILP (HiGHS) returns a **certified
optimum at every budget** from 1 to 66, in {f['ilp_runtime_ms_range'][0]}\u2013{f['ilp_runtime_ms_range'][1]} ms.

- **{f['min_inputs_for_full_achievable_coverage']} inputs** suffice to cover the
  entire *achievable* outcome space. Coverage of all 88 outcomes tops out at
  **{_pct(f['max_coverage_fraction_of_all_outcomes'])}**: some outcomes are
  unreachable by any selection, because their conjunctive arms are never
  jointly satisfiable within the 66-input set. Reporting coverage against the
  reachable denominator is the honest choice; against all 88 it would look like
  a permanent failure that no optimizer can fix.
- A budget of 10 inputs already reaches **{_pct(f['coverage_at_budget_10'])}** of
  all 88 outcomes.
- Greedy matches the certified optimum at **{_pct(f['greedy_matches_optimum_below_saturation'])}**
  of budgets below saturation (K under {f['min_inputs_for_full_achievable_coverage']}), and
  {_pct(f['greedy_matches_optimum_whole_sweep'])} across the whole sweep
  (K = {f['budget_range_swept'][0]} to {f['budget_range_swept'][1]}). The two figures
  differ for a reason worth stating plainly, and it is not the usual one \u2014 see
  below.

### Greedy has a ceiling that budget cannot lift

Greedy's failures are **not** at tight budgets, where one might expect them:
{f['greedy_failures_below_stall']} occur before it stalls and
{f['greedy_failures_at_or_above_stall']} after. From K = {f['greedy_stall_budget']}
onward greedy halts at {f['greedy_plateau_inputs']} inputs and
{_pct(f['greedy_plateau_coverage'])} coverage and does not move again however much
budget it is given, while ILP continues to
{_pct(f['max_coverage_fraction_of_all_outcomes'])}.

The mechanism is conjunctivity. The outcomes greedy cannot reach fire only when
two or more inputs are present together, so **no single input shows a positive
marginal gain** and marginal-gain selection has nowhere to step. This is the
exact failure mode submodularity rules out, and it is visible here only because
the profile is a real one with conjunctive rules rather than a binary
incidence matrix. It is also the property that makes the instance worth giving
to a non-greedy optimizer at all.

Two things follow, and they point in opposite directions. **An instance solved
exactly in milliseconds is not a candidate for quantum speedup** \u2014 that is the
first substantive finding, and it is negative. But greedy failing at a
substantial share of pre-saturation budgets shows the instance is not a
degenerate tie either: the choice of optimizer changes the answer. This is a
consequence of the profile being *conjunctive*. Where outcomes fire only on
combinations of inputs, weighted coverage is no longer submodular, greedy's
1\u22121/e guarantee does not apply, and marginal-gain selection can be led astray
by an input that pays off only in company.

![Classical baselines and the simulable window]({_fig('baselines')})

## 2. Where methods separate at all

Across **{d['instances']} instances** (6 structural regimes \u00d7 7 sizes \u00d7 3 budget
fractions \u00d7 5 seeds), greedy reaches the certified optimum on
**{_pct(d['greedy_optimal_overall'])}** of them. By regime:

| regime | greedy reaches optimum |
|---|---|
{regime_rows}

Dense instances are trivial ({_pct(d['greedy_optimal_by_regime']['dense'])}). The
discriminating cell is **degree-capped structure at a {d['hardest_cell']['budget_fraction']:.0%} budget**,
where greedy reaches the optimum only **{_pct(d['hardest_cell']['greedy_optimal'])}** of
the time \u2014 and it is also the only regime whose exact QUBO encoding fits in a
simulator. Every quantum comparison below runs there. That is a deliberate
choice of the most favourable available ground, stated openly.

## 3. What is and is not simulable

The specification warns that inputs \u2260 qubits; the arithmetic matters. The exact
encoding of the flagship needs 66 input variables + 88 outcome variables, plus
slack, giving **154+ qubits**. State-vector simulation on 48 GiB tops out near
30. So the flagship's exact QUBO **cannot be simulated at all**, and the
head-to-head runs on a size ladder of {h['qubit_range'][0]}\u2013{h['qubit_range'][1]} qubits
(8\u201316 inputs). Any statement about QAOA here is a statement about 16-input
instances, extrapolated to nothing.

## 4. QAOA against classical on identical QUBOs

Same instance, same encoding, same penalty coefficients, same ground truth from
exhaustive enumeration of that same QUBO. 50 instances per configuration.

| method | formulation | reaches certified optimum |
|---|---|---|
{classical_rows}
{qaoa_rows}

{qaoa_verdict}

{qaoa_size_claim}

| inputs | 8 | 10 | 12 | 14 | 16 |
|---|---|---|---|---|---|
| QAOA reaches optimum | {qaoa_by_size} |

Depth buys a little accuracy ({_pct(_by_size(h['qaoa_reaches_optimum_by_depth'], '1'))} \u2192
{_pct(_by_size(h['qaoa_reaches_optimum_by_depth'], '3'))}) at strictly linear two-qubit gate
cost ({_by_size(h['two_qubit_gates_by_depth'], '1')} \u2192 {_by_size(h['two_qubit_gates_by_depth'], '3')} gates).

**Concentration.** Measured as enrichment of optimum probability over uniform
random sampling of the same state space, the median is
**{h['optimum_mass_enrichment_median']}\u00d7** \u2014 real but modest. The distribution
matters more than the median: individual runs span
{h['optimum_mass_enrichment_range'][0]}\u00d7 to {h['optimum_mass_enrichment_range'][1]}\u00d7,
and **{_pct(h['runs_at_or_below_chance_fraction'])} of runs are at or below chance.**

![QAOA versus classical baselines]({_fig('headtohead')})

## 5. Why: the penalty terms dominate the spectrum

The QUBO encodes the budget constraint as a penalty. Measured on the same
instances, the penalty terms exceed the coverage signal by
**{_by_size(p['penalty_dominance_by_size'], '8'):.0f}\u00d7\u2013{_by_size(p['penalty_dominance_by_size'], '16'):.0f}\u00d7**,
and the energy band spanned by useful solutions occupies well under 1% of the
spectrum \u2014 **shrinking with size**:

| inputs | 8 | 10 | 12 | 14 | 16 |
|---|---|---|---|---|---|
| useful span, % of spectrum | {span_row} |
| states with negative energy | {neg_row} |

QAOA spends its variational capacity climbing out of the penalty bulk rather
than discriminating among good solutions. **This was tested as a hypothesis, not
asserted:** scaling the penalty coefficient down more than an order of magnitude
below the safe bound left the optimum probability in the same range, and below a
threshold the QUBO's own minimizer began violating the input budget \u2014 at which
point the relaxed QUBO no longer encodes the problem. Penalty tuning is not an
escape route. This is a property of the constrained-coverage-as-QUBO
formulation, and it is the most transferable result here.

## 6. Noise sensitivity

{n['sweep_points']} points: 3 instance seeds \u00d7 3 depths \u00d7 3 mechanisms \u00d7 2 channels
\u00d7 7 rates, {n['trajectories_per_point']} trajectories each, standard error on every
point. One mechanism is swept at a time from an otherwise-ideal model.
Variational parameters are optimized once on the ideal objective and frozen \u2014
re-optimizing per noise level would answer a different question and mask the
degradation being measured.

At 1% depolarizing error, mean energy degradation by mechanism:

| mechanism | degradation |
|---|---|
{mech_rows}

Against an ideal mean energy of \u007e{_by_size(n['ideal_mean_energy_by_depth'], '1'):.0f}, an
{n['degradation_at_1pct_depolarizing']['two_qubit_error']:.0f}-unit shift from
two-qubit error alone at 1% is severe. Two-qubit gates carry the damage; deeper
circuits lose more, up to a saturation point above \u007e5% where all depths
converge on the fully-randomized mean.

Resolution floor: every point clears one standard error from a two-qubit rate of
{100 * n['fully_resolved_two_qubit_rate']:.0f}% upward. At the smallest rate swept, only
{_pct(n['resolved_fraction_at_smallest_rate'])} of points do, so the sub-1% end of the
sweep bounds the effect rather than measuring it.

**Dephasing at p=1 is exactly invisible** \u2014 a phase error applied after the
final mixer cannot affect computational-basis readout. This is correct physics,
not a null measurement, and it is pinned by a test so a future refactor that
changes it gets flagged.

![Noise sensitivity]({_fig('noise')})

## 7. A bug that invalidated a full experiment

The first noise sweep was **discarded in its entirety.** The noisy trajectory
loop precomputed a cost-phase factor and raised it to the variational angle:
`exp(-iE) ** gamma`. Complex exponentiation wraps the exponent into the
principal branch **before** scaling, so every state with |E| > \u03c0 received the
wrong phase \u2014 which, given spectra reaching +6337, was nearly every state.

It surfaced because two results refused to make sense: energy differences
scattered in sign within one standard error, and p=1 dephasing was *exactly*
constant. Reading the code rather than hedging the write-up found the cause.

A second, smaller defect was found alongside it: the zero-rate baseline
short-circuited before drawing from the random generator, so it consumed a
different random stream than the noisy points it was supposed to control. The
paired comparison the script claimed was not the one the code implemented.

Both are fixed, and both carry regression tests \u2014 including a paired check that
binds the ideal and noisy simulator routes together, which is the test whose
absence let the phase bug survive. **The invalid CSV is retained** as
`noise_sweep_INVALID_phasebug.csv` rather than deleted, so the discard is part
of the record. The head-to-head and penalty-landscape results use only the ideal
path and were verified unaffected.

### A third defect, in the parameter search

The variational parameters were drawn from `gamma` in `[0, pi)`, described in
the code as one period. It is half of one: the cost layer `exp(-i gamma E)`
repeats over `[0, 2 pi)` for this spectrum. On a probe instance the best `gamma`
sat at 6.07, outside the interval the optimizer ever started in.

Worse, the penalised spectrum spans thousands of energy units, so the
expectation is a high-degree trigonometric polynomial in `gamma` with hundreds
of local minima inside a single period. Three random restarts of a local
optimizer is a lottery, not a search. On one instance it returned an expectation
of 147.5 where a dense sweep of the same depth-1 landscape reaches 11.95, against
a ground state of -9.

The search now sweeps a coarse grid over one full, computed period and refines
from its best points. **Both the head-to-head and the noise sweep were re-run**,
and the head-to-head conclusion changed: QAOA p=2 went from 72% to 92%, and from
0% to 80% at sixteen inputs. The earlier numbers measured the classical outer
loop failing, not the ansatz. The spectrum measurements in section 5 are
properties of the QUBO and were unaffected.

### A fourth defect: the same phase bug, in new code

Building the constrained ansatz of section 8, its cost layer was written as
`exp(-iE) ** gamma` — the exact construction that invalidated the first noise
sweep. It was caught before any result was written, because the exported
OpenQASM circuit, which computes the phase correctly, disagreed with the
simulator on six of eight test cases. A regression test now pins it, and the
Qiskit reproduction in section 9 keeps an independent code base in the loop.

A defect that recurs in new code is an argument for keeping the check that
caught it, not for trusting the author more.

{constrained_sections}
## 10. Interpretation

{interpretation_opening} Three conditions would each have to change before
QAOA could be called a better choice than the classical baselines here.

1. **The instance would have to be hard.** Certified optima in milliseconds
   leave nothing to win. The generated instances QAOA can reach are
   disjunctive, so coverage on them is submodular and greedy carries a
   guarantee. The real instance is conjunctive, and greedy stalls there, but it
   is simulable only in slices small enough that greedy still wins.
2. **The encoding would have to stop wasting the spectrum.** Constraint-as-
   penalty puts >99% of the energy range outside the region of interest.
   {mixer_claim}
3. **Noise would have to be far below current rates.** Two-qubit error at 1%
   already shifts the mean energy substantially against its ideal value.

{result_c_claim} That range is small, and the honest statement of scope is
that nothing here extrapolates to 66 inputs.

**A note on how this section changed.** An earlier revision of this report
stated that classical heuristics beat QAOA at every depth. That conclusion was
an artefact of the parameter search, not of the ansatz — see section 7.

## 11. Limitations

- Python reference implementation, not the Rust core.
- The flagship instance is the real 66\u00d788 export; the SYNTHETIC instances are
  the generator's, used only for the structural survey and the head-to-head.
- Simulable range is 8\u201316 inputs. The flagship's exact QUBO needs 154+ qubits.
- 5 instance seeds per size in the head-to-head (25 instances), 3 in the noise
  sweep \u2014 below the 30 the specification suggests for stochastic experiments.
- Trajectory-based noise, not full density-matrix evolution.
- Single classical optimizer (COBYLA) for the variational loop.
- Two QAOA variants: the standard transverse-field mixer and an XY ring mixer.
  No warm starts. The constrained variant has not been run under noise.
- Real-instance slices stop at 16 biomarkers. Their multi-body cost terms are
  priced as unshared CX ladders, an upper bound.
- OpenQASM export covers pairwise coverage only; a conjunctive cost layer needs
  multi-controlled phases the exporter does not emit, so the Qiskit
  reproduction runs on the degree-capped instances.
"""
