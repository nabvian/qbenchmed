"""Export to standard formats, so the benchmark is usable without this package.

Two things leave the project here.

*The problem*, as a CPLEX LP file.  LP is the plain-text format that
qiskit-optimization's ``QuadraticProgram``, CPLEX, Gurobi, SCIP and HiGHS all
read.  Anyone can take a Q-BenchMed instance to their own solver without
importing a line of this code.  :func:`lp_to_qubo_matrix` parses the file back
so the round trip is checked, not assumed.

*The circuits*, as OpenQASM 2.0.  Both ansatzes are emitted -- the penalty
QAOA the head-to-head uses, and the constrained XY-mixer QAOA -- with the
optimised angles baked in and a measurement on every qubit, ready to submit to
hardware.  Every gate is drawn from ``{h, x, rx, ry, rz, cx}``, the universal
set every OpenQASM 2.0 toolchain supports, so nothing depends on an extended
include file.

Each circuit is built once, as a list of gates, and that one list is both
emitted as QASM and applied to this project's own simulator.  The tests load
the emitted QASM into Qiskit and require its statevector to match the
simulator's, so an export that encodes the wrong circuit fails rather than
shipping.

Two decompositions are deliberately unoptimised and say so.  The XY gate is
emitted as its XX and YY halves, four CNOTs, which is correct and easy to audit;
a transpiler reduces it to the optimal two, and that two is what
:mod:`qbm.constrained` charges.  The Dicke preparation follows
Baertschi-Eidenbenz (2019) with its controlled rotations expanded into CNOT
and RY.  Neither choice affects correctness; both are checked.
"""
from __future__ import annotations

import math
import re

import numpy as np

from . import simulator as sim
from .qubo import Qubo

#: One gate: ("h"|"x", q) | ("rx"|"ry"|"rz", theta, q) | ("cx", control, target)
Gate = tuple


# --------------------------------------------------------------------------
# problem export: CPLEX LP
# --------------------------------------------------------------------------
def qubo_to_lp(qubo: Qubo, *, name: str = "qbenchmed") -> str:
    """The QUBO as a CPLEX LP file: ``minimize z^T Q z + offset``, z binary.

    Linear terms are the diagonal of the upper-triangular ``Q``; quadratic
    terms are its strict upper triangle.  LP writes a quadratic objective inside
    ``[ ... ] / 2``, so every quadratic coefficient is doubled to compensate --
    the single most common way to get an LP export silently wrong, and the
    reason :func:`lp_to_qubo_matrix` exists.
    """
    Q = qubo.Q
    n = qubo.n_vars
    var = [f"x{i}" for i in range(n)]

    def term(coef: float, body: str) -> str:
        sign = "-" if coef < 0 else "+"
        return f" {sign} {abs(coef):.12g} {body}"

    linear = "".join(term(Q[i, i], var[i]) for i in range(n) if Q[i, i] != 0)
    quad = "".join(term(2.0 * Q[i, j], f"{var[i]} * {var[j]}")
                   for i in range(n) for j in range(i + 1, n) if Q[i, j] != 0)

    lines = [f"\\ Problem name: {name}",
             f"\\ QUBO with {n} binary variables. Constant offset {qubo.offset:.12g}",
             "\\ is not representable in LP and is recorded here only.",
             "Minimize",
             f" obj:{linear or ' 0 ' + var[0]}"
             + (f" + [{quad} ] / 2" if quad else ""),
             "Subject To",
             "Bounds",
             *[f" 0 <= {v} <= 1" for v in var],
             "Binaries",
             " " + " ".join(var),
             "End", ""]
    return "\n".join(lines)


_LP_TERM = re.compile(r"([+-])\s*([0-9.eE+-]+)\s+(x\d+)(?:\s*\*\s*(x\d+))?")


def lp_to_qubo_matrix(lp: str, n: int) -> np.ndarray:
    """Parse the objective of an LP written by :func:`qubo_to_lp` back into Q.

    Deliberately minimal: it reads exactly the format this module writes, and
    exists so the export is verified by a round trip rather than trusted.
    """
    obj = lp.split("Minimize", 1)[1].split("Subject To", 1)[0]
    if "[" in obj:
        lin_part, rest = obj.split("[", 1)
        quad_part = rest.split("]", 1)[0]
    else:
        lin_part, quad_part = obj, ""
    Q = np.zeros((n, n))
    for sign, coef, a, b in _LP_TERM.findall(lin_part.split(":", 1)[1]):
        if not b:
            Q[int(a[1:]), int(a[1:])] += (1 if sign == "+" else -1) * float(coef)
    for sign, coef, a, b in _LP_TERM.findall(quad_part):
        if b:
            i, j = sorted((int(a[1:]), int(b[1:])))
            Q[i, j] += (1 if sign == "+" else -1) * float(coef) / 2.0
    return Q


# --------------------------------------------------------------------------
# gate building blocks, all over {h, x, rx, ry, rz, cx}
# --------------------------------------------------------------------------
def _rzz(theta: float, a: int, b: int) -> list[Gate]:
    """exp(-i theta Z_a Z_b / 2) as CX-RZ-CX."""
    return [("cx", a, b), ("rz", theta, b), ("cx", a, b)]


def _cry(theta: float, c: int, t: int) -> list[Gate]:
    """Controlled RY(theta), control c, target t."""
    return [("ry", theta / 2, t), ("cx", c, t), ("ry", -theta / 2, t), ("cx", c, t)]


def _ccry(theta: float, c1: int, c2: int, t: int) -> list[Gate]:
    """Doubly-controlled RY(theta): C-V, CX, C-V-dagger, CX, C-V with V = RY(theta/2)."""
    return (_cry(theta / 2, c2, t) + [("cx", c1, c2)] + _cry(-theta / 2, c2, t)
            + [("cx", c1, c2)] + _cry(theta / 2, c1, t))


def _xy(theta: float, a: int, b: int) -> list[Gate]:
    """exp(-i theta (X_a X_b + Y_a Y_b) / 2), the XY mixer's two-qubit term.

    XX and YY commute, so this is exp(-i theta XX/2) exp(-i theta YY/2).  Each
    half is an RZZ in a rotated basis: H maps X to Z, RX(pi/2) maps Y to Z.
    Four CNOTs -- correct and auditable, not minimal; see the module docstring.
    """
    xx = [("h", a), ("h", b)] + _rzz(theta, a, b) + [("h", a), ("h", b)]
    yy = ([("rx", math.pi / 2, a), ("rx", math.pi / 2, b)] + _rzz(theta, a, b)
          + [("rx", -math.pi / 2, a), ("rx", -math.pi / 2, b)])
    return xx + yy


def _scs(n: int, k: int, offset: int = 0) -> list[Gate]:
    """Split-and-cyclic-shift block SCS_{n,k} (Baertschi-Eidenbenz 2019) on qubits 0..n-1."""
    q = lambda i: offset + i                          # noqa: E731
    g: list[Gate] = [("cx", q(n - 2), q(n - 1))]
    g += _cry(2 * math.acos(math.sqrt(1 / n)), q(n - 1), q(n - 2))
    g += [("cx", q(n - 2), q(n - 1))]
    for l in range(2, k + 1):                          # noqa: E741
        g += [("cx", q(n - l - 1), q(n - 1))]
        g += _ccry(2 * math.acos(math.sqrt(l / n)), q(n - 1), q(n - l), q(n - l - 1))
        g += [("cx", q(n - l - 1), q(n - 1))]
    return g


def dicke_gates(n: int, k: int) -> list[Gate]:
    """Deterministic preparation of |D^n_k> from |0>^n (Baertschi-Eidenbenz 2019).

    Starts from ``|0^(n-k) 1^k>`` and applies SCS blocks of shrinking width.
    Verified against a directly constructed Dicke state for every small (n, k)
    in ``tests/test_interop.py``.
    """
    if not 0 <= k <= n:
        raise ValueError(f"weight {k} out of range for {n} qubits")
    g: list[Gate] = [("x", i) for i in range(n - k, n)]
    if k in (0, n):
        return g                                      # |0..0> or |1..1>: already Dicke
    for width in range(n, k, -1):
        g += _scs(width, k)
    for width in range(k, 1, -1):
        g += _scs(width, width - 1)
    return g


def _cost_gates(h: np.ndarray, J: np.ndarray, gamma: float) -> list[Gate]:
    """exp(-i gamma H_C) for H_C = sum h_i Z_i + sum_{i<j} J_ij Z_i Z_j (global phase dropped)."""
    n = len(h)
    g: list[Gate] = [("rz", 2 * gamma * h[i], i) for i in range(n) if h[i] != 0]
    for i in range(n):
        for j in range(i + 1, n):
            if J[i, j] != 0:
                g += _rzz(2 * gamma * J[i, j], i, j)
    return g


# --------------------------------------------------------------------------
# the two ansatzes
# --------------------------------------------------------------------------
def penalty_qaoa_gates(qubo: Qubo, gammas, betas) -> list[Gate]:
    """The head-to-head's penalty QAOA: |+>^n, cost layer, RX(2 beta) mixer."""
    h, J, _ = qubo.to_ising()
    n = qubo.n_vars
    g: list[Gate] = [("h", i) for i in range(n)]
    for gamma, beta in zip(gammas, betas):
        g += _cost_gates(h, J, gamma)
        g += [("rx", 2 * beta, i) for i in range(n)]
    return g


def coverage_qubo(A: np.ndarray, w: np.ndarray) -> Qubo:
    """Coverage objective alone on the n input qubits: no budget term, no slack.

    The constrained ansatz's cost Hamiltonian.  Defined for outcome degree <= 2,
    where coverage is quadratic; a higher-degree outcome needs multi-qubit phase
    terms that an Ising model cannot hold, and is refused rather than truncated.
    """
    A = np.asarray(A, dtype=np.float64)
    w = np.asarray(w, dtype=np.float64)
    n_in, n_out = A.shape
    q = Qubo.zeros(n_in, meta={"mode": "coverage_only"})
    for j in range(n_out):
        idx = np.flatnonzero(A[:, j])
        if idx.size == 1:
            q.add_linear(int(idx[0]), -w[j])
        elif idx.size == 2:
            a, b = int(idx[0]), int(idx[1])
            q.add_linear(a, -w[j])
            q.add_linear(b, -w[j])
            q.add_quadratic(a, b, w[j])
        elif idx.size > 2:
            raise ValueError(
                f"outcome {j} has degree {idx.size}; coverage is not quadratic "
                "there and cannot be written as an Ising cost layer")
    return q


def constrained_qaoa_gates(A: np.ndarray, w: np.ndarray, K: int,
                           gammas, betas) -> list[Gate]:
    """Constrained QAOA: Dicke |D^n_K>, coverage cost layer, XY ring mixer.

    Edge order matches :func:`qbm.constrained.apply_ring_mixer` exactly -- even
    edges, odd edges, then the closing edge -- because a Trotterised mixer's
    result depends on the order its non-commuting terms are applied in.
    """
    q = coverage_qubo(A, w)
    h, J, _ = q.to_ising()
    n = q.n_vars
    g = dicke_gates(n, K)
    for gamma, beta in zip(gammas, betas):
        g += _cost_gates(h, J, gamma)
        for start in (0, 1):
            for i in range(start, n - 1, 2):
                g += _xy(beta, i, i + 1)
        if n > 2:
            g += _xy(beta, n - 1, 0)
    return g


# --------------------------------------------------------------------------
# backends: this simulator, and OpenQASM 2.0
# --------------------------------------------------------------------------
def apply_gates(n: int, gates: list[Gate]) -> sim.StateVector:
    """Run a gate list on this project's simulator from |0>^n."""
    sv = sim.StateVector(n)
    for gate in gates:
        op = gate[0]
        if op == "h":
            sv.apply_1q(sim.H, gate[1])
        elif op == "x":
            sv.apply_1q(sim.X, gate[1])
        elif op == "rx":
            sv.apply_1q(sim.rx(gate[1]), gate[2])
        elif op == "ry":
            sv.apply_1q(sim.ry(gate[1]), gate[2])
        elif op == "rz":
            sv.apply_1q(sim.rz(gate[1]), gate[2])
        elif op == "cx":
            sv.cnot(gate[1], gate[2])
        else:
            raise ValueError(f"unknown gate {op!r}")
    return sv


def to_qasm2(n: int, gates: list[Gate], *, measure: bool = True,
             comment: str | None = None) -> str:
    """Emit a gate list as OpenQASM 2.0 over qelib1.inc's universal subset."""
    lines = ["OPENQASM 2.0;", 'include "qelib1.inc";']
    if comment:
        lines += [f"// {line}" for line in comment.splitlines()]
    lines += [f"qreg q[{n}];", f"creg c[{n}];"]
    for gate in gates:
        op = gate[0]
        if op in ("h", "x"):
            lines.append(f"{op} q[{gate[1]}];")
        elif op in ("rx", "ry", "rz"):
            lines.append(f"{op}({gate[1]:.15g}) q[{gate[2]}];")
        elif op == "cx":
            lines.append(f"cx q[{gate[1]}],q[{gate[2]}];")
        else:
            raise ValueError(f"unknown gate {op!r}")
    if measure:
        lines.append("measure q -> c;")
    return "\n".join(lines) + "\n"


def gate_counts(gates: list[Gate]) -> dict:
    """Two-qubit and total gate counts of an emitted (unoptimised) gate list."""
    two = sum(1 for g in gates if g[0] == "cx")
    return {"cx": two, "total": len(gates)}
