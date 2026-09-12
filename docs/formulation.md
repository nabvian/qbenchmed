# Mathematical formulation

This document derives what `qbm/qubo.py` implements: the native constrained
problems, the three QUBO encodings of the coverage relation, the penalty
coefficient bounds, and a proof that the compact encoding widely used in the
QAOA literature is **not** an exact reformulation of this problem.

Notation follows the project spec. `A[i, j] = 1` means input *i* contributes to
outcome *j*; `w_j` is the weight of outcome *j*; `c_i` the cost of input *i*;
`d_j = Σ_i A[i, j]` the *outcome degree* (how many inputs reach outcome *j*).

---

## 1. Native problems

### 1.1 Maximum coverage under an input budget (spec §9.1)

```
max_x,y   α Σ_j w_j y_j  −  β Σ_i c_i x_i
s.t.      y_j ≤ Σ_i A[i,j] x_i        for all j
          Σ_i x_i ≤ K
          x ∈ {0,1}^n_in,  y ∈ {0,1}^n_out
```

The coverage constraint is one-sided on purpose. It forbids claiming an outcome
no selected input reaches; it does not *force* `y_j = 1` when the outcome is
reached, because the reward term `−α w_j y_j` already makes claiming a reached
outcome strictly profitable whenever `w_j > 0`. Under maximisation the
constraint and the objective together give `y_j = OR_i(A[i,j] x_i)` at any
optimum, which is the intended semantics.

### 1.2 Minimum input set at a coverage floor (spec §9.2)

```
min_x,y   β Σ_i c_i x_i
s.t.      Σ_j w_j y_j ≥ C · Σ_j w_j
          y_j ≤ Σ_i A[i,j] x_i
```

QUBO energies are minimisation-sense throughout, so §1.1's objective enters the
QUBO with its sign flipped. `native_max_coverage_objective` in `qubo.py` returns
this minimisation-sense value and is the **only** definition of ground truth in
the codebase: the QUBO is checked against it, never the reverse.

---

## 2. Inequality constraints need slack

A budget `Σ_i x_i ≤ K` is often penalised as `γ (Σ_i x_i − K)²`. That is wrong
here: it is minimised at `Σ x_i = K` and therefore *penalises* a panel that
achieves the same coverage with fewer inputs. Since "smaller panel, same
coverage" is precisely the answer the benchmark exists to find, the squared-
equality form would bias every algorithm identically and invisibly.

The implementation converts the inequality to an equality with a binary-encoded
slack variable `s ∈ [0, K]`:

```
Σ_i x_i + s = K,     s = Σ_{b=0}^{B−1} 2^b s_b,     B = ⌈log2(K+1)⌉
```

penalised as `γ (Σ_i x_i + Σ_b 2^b s_b − K)²`. Expanding the square gives the
linear and quadratic coefficients in `_budget_block`.

---

## 3. Three encodings of the coverage relation

### 3.1 `slack` — exact at any degree

Convert `y_j ≤ Σ_i A[i,j] x_i` to the equality

```
Σ_i A[i,j] x_i − y_j − s_j = 0,     s_j ∈ [0, d_j − 1]
```

binary-encode `s_j` in `⌈log2 d_j⌉` bits, and penalise the squared residual with
coefficient `λ`. Exact for arbitrary `d_j`, at the cost of `n_out + Σ_j ⌈log2
d_j⌉` auxiliary variables on top of `n_in`.

### 3.2 `pairwise` — exact, `x`-only, requires `d_j ≤ 2`

Eliminate `y` by substituting the coverage indicator directly into the
objective. For an outcome reached by inputs *a* and *b*,

```
OR(x_a, x_b) = x_a + x_b − x_a x_b
```

which is exactly quadratic. Degree-1 outcomes give `x_a`; degree-0 outcomes are
unreachable and contribute a constant. No `y`, no coverage slack: the variable
count is `n_in + ⌈log2(K+1)⌉`.

For `d_j = 3` the substitution is

```
OR(x_a,x_b,x_c) = x_a+x_b+x_c − x_ax_b − x_ax_c − x_bx_c + x_ax_bx_c
```

whose cubic term cannot be dropped without changing the function. Hence the
encoding is exact **iff** every outcome degree is ≤ 2 — which is the entire
reason the `degree_capped` instance regime exists, and why its cap is enforced
by construction rather than by a repair pass.

### 3.3 `penalty` — compact and inexact

The compact form seen throughout the QAOA applications literature keeps `y` and
adds no auxiliaries:

```
λ Σ_j y_j (1 − Σ_i A[i,j] x_i)
```

For `d_j = 1` this is the exact indicator penalty. For `d_j ≥ 2` it is not: when
`y_j = 1` and *m* of the reaching inputs are selected, the term evaluates to
`λ(1 − m)`, i.e. a **spurious reward of λ(m − 1)** for redundant selection.
Redundancy is exactly what the coverage objective is supposed to discourage, so
the distortion is aligned against the quantity being optimised.

### 3.4 The compact encoding cannot be repaired by choosing coefficients

A common response is that `λ` was merely too small. It was not — no coefficient
works, because no *quadratic* penalty in `(x, y)` exists at all.

**Claim.** Let outcome *j* be reached by inputs `S`, `|S| = d ≥ 2`. There is no
quadratic polynomial `P(y_j, x_S)` that vanishes on every feasible assignment
and is strictly positive on the single violating assignment `(y_j = 1, x_S = 0)`.

**Proof.** Write the general quadratic

```
P = c₀ + c₁y + Σ_i a_i x_i + Σ_i e_i y x_i + Σ_{i<k} f_{ik} x_i x_k
```

Every assignment with `y = 0` is feasible, so `P = 0` there for all `x_S`, which
forces `c₀ = 0`, all `a_i = 0`, and all `f_{ik} = 0`. What remains is
`P = c₁y + Σ_i e_i y x_i`. Each assignment with `y = 1` and exactly one
`x_i = 1` is feasible, giving `c₁ + e_i = 0`, so `e_i = −c₁` for every *i*. The
assignment `y = 1` with all `d` inputs selected is also feasible, giving
`c₁ + Σ_i e_i = c₁(1 − d) = 0`. Since `d ≥ 2`, `c₁ = 0`, hence every coefficient
vanishes and `P ≡ 0`. Then `P` is zero on the violating assignment too, not
positive. ∎

The exact penalty is `λ y_j ∏_{i∈S}(1 − x_i)`, of degree `d + 1 ≥ 3`. Reducing
it to quadratic form requires auxiliary variables — which is what §3.1 does.
Exactness at arbitrary outcome degree therefore *costs* qubits; it is not a
matter of tuning.

`exact_quadratic_penalty_exists` in the step-2 test suite verifies the claim
numerically by solving the linear system for `d = 1..5`: solvable only at
`d = 1`.

### 3.5 Measured consequence

Enumerating all assignments for 6-input / 7-outcome instances across all six
structure regimes, 12 seeds, and penalty scales spanning 200× (`λ` from 0.5× to
100× the safe bound):

| `λ` scale | optimum-recovery rate | feasible-solution rate |
|---|---|---|
| 0.5× | 0.097 | 0.444 |
| 1× | 0.097 | 0.444 |
| 2× | 0.097 | 0.444 |
| 10× | 0.097 | 0.444 |
| 100× | 0.097 | 0.444 |

Flat in `λ`, as the proof predicts. The compact encoding recovers the true
optimum about 10% of the time and returns a solution violating the input budget
more than half the time. Both exact encodings recover it in 100% of the same
trials.

**Consequence for this benchmark:** results obtained with the compact encoding
would measure the encoding's distortion, not the optimiser. `penalty` is
retained in the codebase only to quantify that distortion, and is excluded from
every comparative experiment.

---

## 4. Penalty coefficient bounds

A penalty is *safe* when no constraint violation can be profitable. For the
coverage constraints the largest achievable objective gain from falsely claiming
outcomes is `α Σ_j w_j`, and from ignoring input cost is `β Σ_i c_i`. Setting

```
λ > α Σ_j w_j + β Σ_i c_i
```

(implemented as `penalty_bound`, with `+1` for strictness) guarantees that
paying one unit of penalty is never recovered by the objective. The same bound
is used for `γ` on the budget constraint.

This is deliberately conservative. A tighter instance-specific bound exists —
per-constraint, the maximum gain from violating *that* constraint — but a
too-small penalty yields silently infeasible optima, the failure mode the spec's
§41 regression list is written to catch. The cost of conservatism is a larger
dynamic range in the QUBO coefficients, which matters for a noisy QAOA and is
reported alongside the noise results rather than hidden.

---

## 5. QUBO → Ising

With `x_i = (1 − z_i)/2`, `z_i ∈ {−1, +1}` (spec §12; spin `+1` ⇔ bit 0), a QUBO
with diagonal `a` and strict upper triangle `B` maps to

```
h_i   = −a_i/2 − (Σ_k B_ik + Σ_k B_ki)/4
J_ij  = B_ij/4
const = Σ_i a_i/2 + Σ_ij B_ij/4 + offset
```

so that `QUBO(x) = Σ_i h_i z_i + Σ_{i<j} J_ij z_i z_j + const` for **every**
assignment. `to_ising` implements this and the equivalence is asserted
exhaustively in the test suite (spec §35).

Because the cost Hamiltonian is diagonal in the computational basis, the QAOA
cost layer `exp(−iγH_C)` is a phase multiplication over the statevector rather
than a gate sequence — the property that makes the QAOA experiments in step 6
tractable at all. Gate-level counts are still reported, computed from the
`RZZ`/`RZ` decomposition the Hamiltonian implies, so the resource metrics of
spec §43 remain meaningful for hardware discussion.

---

## 6. Qubit cost of exactness

Variable counts for a `heme_shaped` instance (mean outcome degree ≈ 4.7),
`K = ⌈n_in/4⌉`:

| encoding | variables | exact? |
|---|---|---|
| `pairwise` | `n_in + ⌈log2(K+1)⌉` | only if max degree ≤ 2 |
| `penalty` | `n_in + n_out + ⌈log2(K+1)⌉` | never (§3.4) |
| `slack` | `n_in + n_out + Σ_j⌈log2 d_j⌉ + ⌈log2(K+1)⌉` | yes |

At the flagship 66×88 size the exact `slack` encoding needs on the order of
`66 + 88 + ~190 + 5 ≈ 350` variables. Statevector simulation on this machine
(48 GiB, complex128) reaches about 30 qubits. The gap is roughly ten orders of
magnitude in state space, and it is not an implementation deficiency: it is what
exact QUBO reformulation of this problem costs. `fig_encoding_size.png` draws
the simulation wall across the encodings.
