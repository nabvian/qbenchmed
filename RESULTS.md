# Results

Summary of what the benchmark has measured. The full report, regenerated from
the result tables, is in
[reproducibility/REPORT.md](reproducibility/REPORT.md). The tables themselves
are in [results/](results/), one CSV per experiment with the environment and
measured runtime recorded alongside.

Every figure below came from a run in this repository.

## QAOA against classical solvers, on the identical QUBO

Same instance, same encoding, same penalty coefficients, ground truth from
exhaustive enumeration of that same QUBO. Five sizes from eight to sixteen
inputs, five seeds each, twenty-five instances per configuration.

| method | formulation | reaches the certified optimum |
|---|---|---|
| exhaustive | QUBO | 100% |
| tabu | QUBO | 100% |
| QAOA p=2 | QUBO | 92% |
| annealing | QUBO | 88% |
| greedy | native | 80% |
| QAOA p=3 | QUBO | 72% |
| QAOA p=1 | QUBO | 60% |

QAOA at depth two sits above annealing and greedy and below tabu. With
twenty-five instances per configuration those rates are not separated by the
data: 92% carries a 95% interval of [77%, 98%], annealing's 88% carries
[71%, 97%]. The honest statement is that QAOA at its best depth is comparable to
the classical heuristics on this instance family, not better than them.

Tabu solves every instance.

## The result that changed

An earlier revision of this repository reported that classical heuristics beat
QAOA at every depth, with p=2 at 72%. That was an artefact of the parameter
search rather than a property of the ansatz.

Two defects compounded. The variational parameter gamma was drawn from `[0, pi)`
and documented as one period; the period for this spectrum is `[0, 2 pi)`, and on
a probe instance the best value sat at 6.07, outside the interval the optimiser
ever started in. And the penalised spectrum spans thousands of energy units, so
the expectation is a high-degree trigonometric polynomial in gamma carrying
hundreds of local minima inside one period. Three random restarts of a local
optimiser was a lottery.

On one instance the old search returned an expectation of 147.5 where a dense
sweep of the same depth-1 landscape reaches 11.95, against a ground state of -9.

The search now sweeps a coarse grid over one computed period and refines from
its best points. Both affected experiments were re-run:

| | before | after |
|---|---|---|
| QAOA p=1 | 64% | 60% |
| QAOA p=2 | 72% | **92%** |
| QAOA p=3 | 64% | 72% |
| p=2 at sixteen inputs | 0% | **80%** |

Section 7 of the report records this alongside the two earlier defects, one of
which invalidated a whole noise sweep. The invalid CSV is retained rather than
deleted.

## Why the encoding is the problem

This result survived the correction, because it is a property of the QUBO rather
than of any solver.

The budget constraint is expressed as a penalty. Measured on the same instances,
the penalty terms exceed the coverage signal by **85 to 374 times**, and the
energy band spanned by useful solutions occupies **under 1% of the spectrum** —
shrinking as the problem grows, from 0.91% at eight inputs to 0.24% at sixteen.

QAOA spends its variational capacity climbing out of the penalty bulk rather
than discriminating among good solutions. This was tested rather than asserted:
scaling the penalty coefficient down by more than an order of magnitude left the
optimum probability in the same range, and below a threshold the QUBO's own
minimiser began violating the input budget, at which point it no longer encodes
the problem.

There is a second symptom worth recording. Fixing the parameter search improved
the quantity QAOA optimises in every probe case, by a mean of 112 energy units,
while often making it *less* likely to sample the optimum — 0.00573 to 0.00073
in one case. Under a penalty encoding the variational objective and the thing
you actually need come apart.

## The prediction, tested: a constrained mixer

If the penalty is the problem, removing it should help. The constrained ansatz
never leaves the set of valid panels: it starts from the Dicke state, the equal
superposition of every panel with exactly K biomarkers, mixes with an XY ring
that conserves the number of selected biomarkers, and uses coverage alone as its
cost, with no penalty and no slack qubits.

Run on the same 25 instances as the head-to-head, same shots, same restarts,
same parameter search. The fair baseline is a random valid panel:

| depth | penalty QAOA, vs random | constrained QAOA, vs random |
|---|---|---|
| 1 | 0.02× | 5.0× |
| 2 | 0.26× | 9.1× |
| 3 | 0.05× | 13.3× |

Medians. **The constrained ansatz beats a random valid panel on every instance
at every depth**, and its margin is largest at the largest size: 2.5× at
eight inputs, 17× at sixteen, though not rising steadily in between (it dips
at fourteen). **The penalty ansatz does worse than random guessing**, because
it spends probability on panels that break the budget.

It also runs on fewer qubits (16 against 19 at the largest size), and every
sample it produces is valid.

Two things it does not show. The rate at which any of 4,096 shots hits the
optimum is 100% for the constrained ansatz, but random guessing among valid
panels does that too when there are at most 4,368 of them, so that number is
left out of the claim. And the Dicke state has to be prepared: counting the
real preparation circuit, the constrained version costs more two-qubit gates at
depth 1, breaks even at depth 2 (414 against 418), and is cheaper at depth 3.
On a device, where two-qubit error dominates, that trade is the open question.

## On the real instance

The head-to-head uses generated instances because the penalty encoding of the
real one cannot be simulated. Its rules need several biomarkers together, and a
penalty QUBO spends an auxiliary variable per outcome plus slack to say so:

| biomarkers | 10 | 12 | 14 | 16 |
|---|---|---|---|---|
| qubits, penalty encoding | 85 | 100 | 110 | 120 |
| qubits, constrained | 10 | 12 | 14 | 16 |
| constrained vs random, best depth | 29.5× | 12.3× | 12.6× | 63.5× |

These are nested slices of QBMED-HEME-001, keeping every rule whose biomarkers
all survive. The constrained ansatz runs on them where the penalty encoding
cannot, and beats a random valid panel by 4.6× to 63.5× across depths.

At these sizes greedy still finds the optimum, so QAOA is not beating anything
classical here. And depth is not reliably helpful: at 12 and 14 biomarkers,
depth 3 did worse than depth 2, which points at the parameter search.

## Reproduced on a second platform

Every number above comes from this package's own simulator. To check that the
simulator is not the source of the result, both ansatzes were optimised here,
exported as OpenQASM 2.0, and run by Qiskit's sampler, a separate code base
reading the circuit from text. 18 of 18 runs agree within shot noise, and the
two simulators' states agree to twelve decimal places.

That check earned its place. While building the constrained ansatz, its cost
layer was written with exactly the phase bug that once invalidated the noise
sweep. The exported circuit disagreed with the simulator, which is how it was
found, before any result was recorded.

## Noise

279 points: three instance seeds, three depths, three mechanisms, two channels,
seven rates, 400 trajectories each, with a standard error on every point. One
mechanism is swept at a time from an otherwise ideal model. Parameters are
optimised once on the ideal objective and frozen, because re-optimising per
noise level would answer a different question and mask the degradation being
measured.

Mean energy degradation at 1% depolarizing error:

| mechanism | degradation |
|---|---|
| two-qubit error | 28 |
| measurement error | 4 |
| single-qubit error | 4 |

Against an ideal mean energy of about 30, a 28-unit shift from two-qubit error
alone is severe. Two-qubit gates carry the damage.

Single-qubit dephasing at p=1 is exactly invisible, deviation `0.00e+00` across
every rate and seed. A phase error after the final mixer cannot affect
computational-basis readout. That is correct physics rather than a null
measurement, and a test pins it so a refactor that changes it gets flagged.

## The classical side

On the 66-biomarker, 88-outcome instance, the certified solver proves the
optimum at every panel size in about a second, and agrees to the digit with an
independent SciPy/HiGHS solver that shares no code with it:

| | this crate | SciPy/HiGHS |
|---|---|---|
| coverage at K=20 | 79.55% | 79.55% |
| maximum achievable | 96.59% | 96.59% |
| smallest panel, 80% | 21 inputs | 21 inputs |
| smallest panel, 90% | 30 inputs | 30 inputs |

Full coverage is impossible at any budget: three outcomes cannot be reached by
any selection, because their conjunctive arms are never jointly satisfiable
within the 66 inputs. A 100% floor is refused rather than rounded down.

Greedy stalls. From a budget of 34 it halts at 33 inputs and 94.32% and does not
move again however much budget it is given, while the certified solver continues
to 96.59%. The outcomes greedy cannot reach fire only when several inputs are
present together, so no single input shows a positive marginal gain and
marginal-gain selection has nowhere to step. This is the failure mode
submodularity rules out, and it is visible only because the instance is
conjunctive.

## What none of this shows

No quantum advantage. The simulable range stops at sixteen inputs and nothing
here extrapolates past it. The 66-input instance needs 154 or more qubits in its
exact encoding and cannot be simulated at all.

No clinical claim. A covering panel solves a coverage objective over a rule
graph. It does not follow that any biomarker is clinically unnecessary, that a
smaller panel is safe, or that an outcome set is validated because it can be
optimised.

No hardware result. Everything is simulated, noiselessly for the constrained
mixer. The circuits export as standard OpenQASM, but none has been run on a
device.
