"""Noise sensitivity of QAOA on benchmark QUBOs (spec1 s47-48).

Design choices that bound the claims:

* One channel is swept at a time from an otherwise-ideal model, so an effect can
  be attributed to a mechanism rather than to "noise" in general.
* Every point carries a standard error over trajectories, because a trajectory
  mean is an estimate and reporting it bare would overstate precision.
* The ideal point (rate 0) is produced by the same code path as the noisy
  points, so the comparison is not confounded by a different simulator route.
* P(optimum) has a resolution floor of 1/TRAJECTORIES.  Because the ideal
  optimum probability on these instances is ~1e-3, a reported 0.0 means
  "below the floor", not "measured to be zero"; mean energy with its standard
  error is the metric that actually resolves noise effects here.
* Parameters are optimized ONCE on the ideal objective and then held fixed
  across the sweep.  Re-optimizing per noise level would answer a different and
  much more expensive question (noise-aware training) and would hide the
  degradation this experiment is meant to measure.

Run: python experiments/run_noise.py
"""

from __future__ import annotations

import json
import platform
import sys
import time
from pathlib import Path

import numpy as np
import pandas as pd

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from qbm import instances as ins     # noqa: E402
from qbm import noise as nz          # noqa: E402
from qbm import qaoa as qa           # noqa: E402
from qbm import qubo as qb           # noqa: E402

N_INPUTS = 12                # 15 qubits: wide enough to be structured, cheap enough to sweep
DEPTHS = (1, 2, 3)
RATES = (0.0, 0.001, 0.005, 0.01, 0.02, 0.05, 0.1)
CHANNELS = ("depolarizing", "dephasing")
MECHANISMS = ("single_qubit_error", "two_qubit_error", "measurement_error")
TRAJECTORIES = 1500
INSTANCE_SEEDS = (0, 1, 2)
OUT = Path(__file__).resolve().parents[1] / "results"


def main() -> None:
    rows: list[dict] = []
    t0 = time.perf_counter()
    n_out = int(round(N_INPUTS * 88 / 66))
    K = max(2, round(0.4 * N_INPUTS))

    for iseed in INSTANCE_SEEDS:
        inst = ins.generate("degree_capped", N_INPUTS, n_out, seed=iseed)
        q = qb.build_max_coverage(inst.A, inst.w, inst.c, K=K, encoding="pairwise")
        _, E = q.all_energies()
        opt = float(E.min())
        engine = qa.Qaoa(q)

        for p in DEPTHS:
            # optimize once on the ideal objective, then freeze
            rec = engine.optimize(p, seed=iseed, n_restarts=4)
            params = np.asarray(rec.final_parameters)

            for channel in CHANNELS:
                for mech in MECHANISMS:
                    # measurement error is a classical readout flip; the coherent
                    # channel label does not apply, so run it once only
                    if mech == "measurement_error" and channel != CHANNELS[0]:
                        continue
                    for rate in RATES:
                        model = nz.NoiseModel(channel=channel, **{mech: rate})
                        rep = nz.noisy_qaoa_energies(
                            engine, params, model, trajectories=TRAJECTORIES,
                            seed=1000 + iseed, optimum_energy=opt)
                        rows.append(dict(
                            n_inputs=N_INPUTS, qubits=q.n_vars, K=K,
                            instance_seed=iseed, depth=p, channel=channel,
                            mechanism=mech, rate=rate, optimum=opt,
                            mean_energy=rep.mean_energy, sem_energy=rep.sem_energy,
                            best_energy=rep.best_energy,
                            optimum_probability=rep.optimum_probability,
                            realised_errors=rep.realised_error_count,
                            trajectories=TRAJECTORIES,
                            popt_resolution_floor=1.0 / TRAJECTORIES,
                            ideal_expectation=rec.best_expectation))
            print(f"  seed={iseed} p={p} done ({time.perf_counter()-t0:.0f}s)")

    df = pd.DataFrame(rows)
    OUT.mkdir(parents=True, exist_ok=True)
    df.to_csv(OUT / "noise_sweep.csv", index=False)

    env = {"python": sys.version.split()[0], "platform": platform.platform(),
           "n_inputs": N_INPUTS, "depths": list(DEPTHS), "rates": list(RATES),
           "channels": list(CHANNELS), "mechanisms": list(MECHANISMS),
           "trajectories": TRAJECTORIES, "instance_seeds": list(INSTANCE_SEEDS),
           "parameters": "optimized once on the ideal objective, held fixed",
           "total_runtime_s": round(time.perf_counter() - t0, 1)}
    (OUT / "noise_sweep_env.json").write_text(json.dumps(env, indent=2))
    print(f"\n{len(df)} rows -> {OUT/'noise_sweep.csv'}")


if __name__ == "__main__":
    main()
