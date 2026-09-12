"""Regression: the zero-noise baseline must be rng-paired with noisy runs.

Discovered while analysing the noise sweep (spec1 s41 requires a permanent test
per bug).  ``apply_channel`` used to return early on ``rate <= 0`` without
drawing, so an ideal run consumed a different number of random numbers than a
noisy run of the same circuit.  The zero point was therefore not a paired
control, and a pure random-stream offset could masquerade as a noise effect.
"""

from __future__ import annotations

import numpy as np
import pytest

from qbm import instances as ins
from qbm import noise as nz
from qbm import qaoa as qa
from qbm import qubo as qb
from qbm.simulator import StateVector


def test_zero_rate_consumes_one_draw():
    """A zero-rate call must advance the generator exactly like a noisy call."""
    sv = StateVector.plus_state(2)
    r0 = np.random.default_rng(0)
    r1 = np.random.default_rng(0)
    assert nz.apply_channel(sv, 0, 0.0, "dephasing", r0) is False
    r1.random()
    # both generators must now be at the same position
    assert r0.random() == r1.random()


def test_zero_rate_matches_ideal_statevector():
    """An all-zero noise model must reproduce the noiseless energy distribution."""
    inst = ins.generate("degree_capped", 6, 8, seed=0)
    q = qb.build_max_coverage(inst.A, inst.w, inst.c, K=3, encoding="pairwise")
    eng = qa.Qaoa(q)
    params = np.array([0.3, 0.7])

    model = nz.NoiseModel(channel="dephasing")
    assert model.is_ideal
    rep = nz.noisy_qaoa_energies(eng, params, model, trajectories=2000, seed=7)

    # exact noiseless expectation via the engine's own ideal path
    exact = float(eng.expectation(params))
    # 2000 trajectories: allow 4 standard errors
    assert abs(rep.mean_energy - exact) <= 4 * rep.sem_energy + 1e-9


@pytest.mark.parametrize("channel", ["depolarizing", "dephasing"])
def test_zero_rate_reproducible_across_channels(channel):
    """At zero rate the channel label cannot change the result."""
    inst = ins.generate("degree_capped", 6, 8, seed=1)
    q = qb.build_max_coverage(inst.A, inst.w, inst.c, K=3, encoding="pairwise")
    eng = qa.Qaoa(q)
    params = np.array([0.4, 0.6])
    rep = nz.noisy_qaoa_energies(
        eng, params, nz.NoiseModel(channel=channel), trajectories=64, seed=3)
    ref = nz.noisy_qaoa_energies(
        eng, params, nz.NoiseModel(channel="depolarizing"), trajectories=64, seed=3)
    assert rep.mean_energy == pytest.approx(ref.mean_energy)
    assert rep.realised_error_count == 0


def test_cost_phase_survives_large_energies():
    """exp(-i*E)**gamma != exp(-i*gamma*E) once |E| exceeds pi.

    The noisy path once precomputed ``exp(-1j*diag)`` and raised it to
    ``gamma``, which wraps the energy into the principal branch before scaling.
    Penalty-dominated QUBOs reach |E| ~ 1e3, so nearly every state got the wrong
    cost phase and the noisy simulator diverged from the ideal one by ~57 sigma
    while reporting no error.  This pins the identity directly.
    """
    diag = np.array([0.0, 3.0, 316.0, 6337.0])
    for gamma in (0.3, 1.7):
        wrong = np.exp(-1j * diag) ** gamma
        right = np.exp(-1j * gamma * diag)
        # the buggy form must genuinely differ -- otherwise this test is vacuous
        assert np.abs(wrong - right).max() > 1.0
        assert np.allclose(np.exp(-1j * gamma * diag), right)


@pytest.mark.parametrize("p", [1, 2])
def test_ideal_and_noisy_paths_agree(p):
    """Zero-noise trajectories must match the ideal engine's expectation.

    This is the test that catches any divergence between the two simulator
    routes, including the cost-phase bug above.
    """
    inst = ins.generate("degree_capped", 8, 10, seed=4)
    q = qb.build_max_coverage(inst.A, inst.w, inst.c, K=4, encoding="pairwise")
    eng = qa.Qaoa(q)
    rng = np.random.default_rng(0)
    params = rng.uniform(0.1, 1.2, size=2 * p)
    rep = nz.noisy_qaoa_energies(
        eng, params, nz.NoiseModel(channel="dephasing"), trajectories=4000, seed=5)
    exact = float(eng.expectation(params))
    assert abs(rep.mean_energy - exact) <= 4 * rep.sem_energy + 1e-9


def test_dephasing_invisible_at_p1_is_expected():
    """Z errors after the final mixer commute with computational-basis readout.

    This is physics, not a bug: at p=1 the only single-qubit dephasing sites sit
    after the last RX, where a Z error changes phases the measurement cannot
    see.  The test pins the expectation so a future refactor that *does* make
    p=1 dephasing bite gets flagged rather than silently accepted.
    """
    inst = ins.generate("degree_capped", 6, 8, seed=2)
    q = qb.build_max_coverage(inst.A, inst.w, inst.c, K=3, encoding="pairwise")
    eng = qa.Qaoa(q)
    params = np.array([0.35, 0.55])
    base = nz.noisy_qaoa_energies(
        eng, params, nz.NoiseModel(channel="dephasing"), trajectories=128, seed=11)
    for rate in (0.01, 0.1):
        rep = nz.noisy_qaoa_energies(
            eng, params, nz.NoiseModel(channel="dephasing", single_qubit_error=rate),
            trajectories=128, seed=11)
        assert rep.mean_energy == pytest.approx(base.mean_energy)
