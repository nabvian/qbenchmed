"""Reproduce the constrained and penalty results on an independent SDK: Qiskit.

Every number elsewhere in this repository comes from this package's own
state-vector simulator.  That is a single point of failure -- a bug in the
simulator is a bug in every result, and one such bug (a wrapped phase in the
constrained cost layer) was caught only because an exported circuit disagreed
with it.

Here the dependency is removed.  For each instance, the angles are optimised
with this package, both ansatzes are exported to OpenQASM 2.0 with those angles
baked in, and the QASM is run by Qiskit's StatevectorSampler -- a different
code base, parsing the circuit from text.  Its shot counts give an estimate of
the probability of sampling the optimum, which must agree with this package's
exact value within shot noise.  The exact statevector fidelity between the two
is recorded as well.

Requires the ``dev`` extra (qiskit).  Instances are the head-to-head's
degree-capped family, because a conjunctive instance's cost layer needs
multi-controlled phases the export does not emit.

Writes: results/qiskit_reproduction.csv
        results/qiskit_reproduction_env.json
"""
from __future__ import annotations

import itertools
import json
import sys
import time
from pathlib import Path

import numpy as np
import pandas as pd
import qiskit
from qiskit import qasm2
from qiskit.primitives import StatevectorSampler
from qiskit.quantum_info import Statevector

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from qbm import constrained as cn
from qbm import instances as ins
from qbm import interop as io
from qbm import qubo as qb
from qbm import results as rs
from qbm.qaoa import Qaoa

SIZES = (8, 10, 12)
SEEDS = (0, 1, 2)
DEPTH = 2
K_FRAC = 0.30
SHOTS = 20_000
OUT = Path(__file__).resolve().parents[1] / "results"


def _qiskit_state(qasm: str, n: int) -> np.ndarray:
    qc = qasm2.loads(qasm)
    qc.remove_final_measurements()
    vec = np.asarray(Statevector(qc).data)
    out = np.empty_like(vec)
    for idx in range(2 ** n):                          # little-endian -> ours
        out[sum(((idx >> k) & 1) << (n - 1 - k) for k in range(n))] = vec[idx]
    return out


def _qiskit_counts(qasm: str, shots: int, seed: int) -> dict[str, int]:
    job = StatevectorSampler(seed=seed).run([qasm2.loads(qasm)], shots=shots)
    return job.result()[0].data.c.get_counts()


def _bits(key: str, n: int) -> np.ndarray:
    """Qiskit prints qubit 0 rightmost; return bits in variable order."""
    return np.array([int(key[n - 1 - i]) for i in range(n)], dtype=bool)


def main() -> None:
    t_start = time.perf_counter()
    rows: list[dict] = []
    for n_in, seed in itertools.product(SIZES, SEEDS):
        inst = ins.generate("degree_capped", n_in, round(n_in * 88 / 66), seed=seed)
        K = max(2, round(K_FRAC * n_in))

        # constrained
        diag = cn.coverage_diagonal(inst.A, inst.w)
        opt, _ = cn.optimum_at_weight(diag, K)
        qa = cn.ConstrainedQaoa(inst.A, inst.w, K)
        out = qa.run(DEPTH, shots=1, seed=seed, n_restarts=3, optimum_energy=opt)
        g, b = np.array(out.parameters[:DEPTH]), np.array(out.parameters[DEPTH:])
        qasm = io.to_qasm2(n_in, io.constrained_qaoa_gates(inst.A, inst.w, K, g, b))
        ours = qa.state(g, b).vector
        theirs = _qiskit_state(qasm, n_in)
        counts = _qiskit_counts(qasm, SHOTS, seed)
        flat = diag.reshape(-1)
        hits = sum(c for key, c in counts.items()
                   if flat[int("".join(str(int(v)) for v in _bits(key, n_in)), 2)] <= opt + 1e-9)
        feasible = sum(c for key, c in counts.items() if _bits(key, n_in).sum() == K)
        rows.append(_row("constrained", n_in, seed, K, out.optimum_probability,
                         hits, feasible, ours, theirs, qasm))

        # penalty
        q = qb.build_max_coverage(inst.A, inst.w, inst.c, K, encoding="pairwise",
                                  arms=inst.arms)
        _, E = q.all_energies()
        qopt = float(E.min())
        pq = Qaoa(q)
        po = pq.run(DEPTH, shots=1, seed=seed, n_restarts=3, optimum_energy=qopt)
        pg = np.array(po.optimizer.final_parameters[:DEPTH])
        pb = np.array(po.optimizer.final_parameters[DEPTH:])
        qasm = io.to_qasm2(q.n_vars, io.penalty_qaoa_gates(q, pg, pb))
        ours = pq.state(pg, pb).vector
        theirs = _qiskit_state(qasm, q.n_vars)
        counts = _qiskit_counts(qasm, SHOTS, seed)
        hits = sum(c for key, c in counts.items()
                   if q.energy(_bits(key, q.n_vars)) <= qopt + 1e-9)
        feasible = sum(c for key, c in counts.items()
                       if _bits(key, q.n_vars)[:n_in].sum() <= K)
        rows.append(_row("penalty", n_in, seed, K, po.optimum_probability,
                         hits, feasible, ours, theirs, qasm))
        print(f"n={n_in} seed={seed} done ({time.perf_counter()-t_start:.0f}s)", flush=True)

    df = pd.DataFrame(rows)
    OUT.mkdir(parents=True, exist_ok=True)
    df.to_csv(OUT / "qiskit_reproduction.csv", index=False)
    env = {"environment": rs.environment_fingerprint(), "qiskit": qiskit.__version__,
           "sampler": "qiskit.primitives.StatevectorSampler",
           "sizes": list(SIZES), "seeds": list(SEEDS), "depth": DEPTH,
           "shots": SHOTS, "k_frac": K_FRAC,
           "agreement_rule": "|qiskit estimate - exact| <= 4 binomial standard errors",
           "total_runtime_s": round(time.perf_counter() - t_start, 1)}
    (OUT / "qiskit_reproduction_env.json").write_text(json.dumps(env, indent=2))
    print(df[["ansatz", "n_inputs", "seed", "exact_p_opt", "qiskit_p_opt",
              "z_score", "agrees", "state_fidelity"]].round(5).to_string(index=False))
    print(f"\nagree: {int(df.agrees.sum())}/{len(df)}   "
          f"min fidelity: {df.state_fidelity.min():.12f}")


def _row(ansatz, n_in, seed, K, exact, hits, feasible, ours, theirs, qasm) -> dict:
    est = hits / SHOTS
    se = max(np.sqrt(exact * (1 - exact) / SHOTS), 1 / SHOTS)
    fidelity = float(abs(np.vdot(ours, theirs)) ** 2)
    return {"ansatz": ansatz, "n_inputs": n_in, "seed": seed, "K": K,
            "exact_p_opt": exact, "qiskit_p_opt": est,
            "z_score": (est - exact) / se, "agrees": bool(abs(est - exact) <= 4 * se),
            "qiskit_feasible": feasible / SHOTS, "state_fidelity": fidelity,
            "qasm_cx": qasm.count("cx q["), "shots": SHOTS}


if __name__ == "__main__":
    main()
