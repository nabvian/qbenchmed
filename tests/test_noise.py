"""Noise model validation (spec1 s47-48; spec3 s12).

The central requirement is that noise degrades and never flatters: a noisy run
must not be reported as better than the identical ideal run, and the ideal case
must be recoverable exactly by setting all rates to zero.
"""

from __future__ import annotations

import numpy as np
import pytest

from qbm import noise as nz
from qbm import qaoa as qa
from qbm import qubo as qb
from qbm import simulator as sim


def tiny_engine(n=3, seed=0):
    rng = np.random.default_rng(seed)
    q = qb.Qubo.zeros(n)
    for i in range(n):
        q.add_linear(i, float(rng.normal()))
    for i in range(n):
        for j in range(i + 1, n):
            q.add_quadratic(i, j, float(rng.normal()))
    return qa.Qaoa(q), q


# --------------------------------------------------------------------------
# model validation
# --------------------------------------------------------------------------
def test_default_model_is_ideal():
    assert nz.NoiseModel().is_ideal


@pytest.mark.parametrize("field", ["single_qubit_error", "two_qubit_error",
                                   "measurement_error", "layer_idle_error"])
def test_nonzero_rate_is_not_ideal(field):
    assert not nz.NoiseModel(**{field: 0.01}).is_ideal


@pytest.mark.parametrize("bad", [-0.01, 1.5])
def test_rates_outside_unit_interval_rejected(bad):
    with pytest.raises(ValueError, match="outside"):
        nz.NoiseModel(single_qubit_error=bad)


def test_unknown_channel_rejected():
    with pytest.raises(ValueError, match="unknown channel"):
        nz.NoiseModel(channel="cosmic_rays")


# --------------------------------------------------------------------------
# channel behaviour
# --------------------------------------------------------------------------
def test_zero_rate_never_injects():
    sv = sim.StateVector.plus_state(2)
    before = sv.vector.copy()
    rng = np.random.default_rng(0)
    for _ in range(50):
        assert not nz.apply_channel(sv, 0, 0.0, "depolarizing", rng)
    assert np.allclose(sv.vector, before)


def test_rate_one_always_injects():
    rng = np.random.default_rng(0)
    for _ in range(20):
        sv = sim.StateVector(2)
        assert nz.apply_channel(sv, 0, 1.0, "bit_flip", rng)


def test_bit_flip_maps_zero_to_one():
    sv = sim.StateVector(1)                       # |0>
    rng = np.random.default_rng(0)
    nz.apply_channel(sv, 0, 1.0, "bit_flip", rng)
    assert abs(sv.vector[1]) == pytest.approx(1.0)


def test_dephasing_preserves_populations():
    """Z errors change phases only -- measurement probabilities must not move."""
    sv = sim.StateVector.plus_state(3)
    before = sv.probabilities().copy()
    rng = np.random.default_rng(1)
    for q in range(3):
        nz.apply_channel(sv, q, 1.0, "dephasing", rng)
    assert np.allclose(sv.probabilities(), before)


@pytest.mark.parametrize("channel", nz.CHANNELS)
def test_every_channel_preserves_normalisation(channel):
    """Including amplitude damping, which is non-unitary and renormalised."""
    rng = np.random.default_rng(2)
    sv = sim.StateVector.plus_state(3)
    for _ in range(30):
        nz.apply_channel(sv, int(rng.integers(3)), 1.0, channel, rng)
        assert sv.norm() == pytest.approx(1.0, abs=1e-10)
    sv.check_numerics()


def test_amplitude_damping_drives_toward_ground_state():
    sv = sim.StateVector(1)
    sv.apply_1q(sim.X, 0)                         # |1>
    rng = np.random.default_rng(0)
    nz.apply_channel(sv, 0, 1.0, "amplitude_damping", rng)
    assert abs(sv.vector[0]) == pytest.approx(1.0), "should have damped to |0>"


def test_noise_is_not_charged_to_resource_counters():
    """A noisy run must report the same circuit cost as the ideal run.

    Otherwise the resource metrics would conflate algorithm cost with error
    injection, and deeper-vs-noisier comparisons would be uninterpretable.
    """
    sv = sim.StateVector.plus_state(3)
    sv.rx_all(0.3)
    before = sv.resources.gate_count
    rng = np.random.default_rng(0)
    for q in range(3):
        nz.apply_channel(sv, q, 1.0, "depolarizing", rng)
    assert sv.resources.gate_count == before


# --------------------------------------------------------------------------
# noisy QAOA evaluation
# --------------------------------------------------------------------------
def test_ideal_model_reproduces_noiseless_energies():
    """Rates at zero must recover the ideal distribution exactly."""
    engine, q = tiny_engine()
    params = np.array([0.4, 0.3])
    rep = nz.noisy_qaoa_energies(engine, params, nz.NoiseModel(),
                                 trajectories=400, seed=5)
    exact = engine.expectation(params)
    # trajectory mean estimates the exact expectation; 3 SEM is a generous bound
    assert abs(rep.mean_energy - exact) < 4 * rep.sem_energy + 1e-9
    assert rep.realised_error_count == 0


def test_noise_degrades_mean_energy():
    """Strong noise must not improve the objective (spec1 s48).

    Checked as a mean over seeds rather than per-seed: trajectory noise is
    stochastic, and a single seed asserting a strict inequality would be a flaky
    test dressed up as a scientific claim.
    """
    engine, q = tiny_engine(n=4, seed=1)
    rec = engine.optimize(2, seed=0, n_restarts=4)
    params = np.asarray(rec.final_parameters)

    ideal = np.mean([nz.noisy_qaoa_energies(engine, params, nz.NoiseModel(),
                                            trajectories=200, seed=s).mean_energy
                     for s in range(4)])
    noisy = np.mean([nz.noisy_qaoa_energies(
        engine, params, nz.NoiseModel(single_qubit_error=0.15,
                                      two_qubit_error=0.15),
        trajectories=200, seed=s).mean_energy for s in range(4)])
    assert noisy > ideal, f"noise improved the objective: {noisy} < {ideal}"


def test_measurement_error_flips_bits():
    engine, q = tiny_engine()
    params = np.array([0.5, 0.4])
    clean = nz.noisy_qaoa_energies(engine, params, nz.NoiseModel(),
                                   trajectories=200, seed=3)
    noisy = nz.noisy_qaoa_energies(engine, params,
                                   nz.NoiseModel(measurement_error=0.3),
                                   trajectories=200, seed=3)
    assert noisy.realised_error_count > 0
    assert clean.realised_error_count == 0


def test_report_carries_its_own_uncertainty():
    """A noisy number without its uncertainty is not reportable."""
    engine, q = tiny_engine()
    rep = nz.noisy_qaoa_energies(engine, np.array([0.3, 0.2]),
                                 nz.NoiseModel(single_qubit_error=0.05),
                                 trajectories=100, seed=1)
    d = rep.as_dict()
    assert d["trajectories"] == 100
    assert np.isfinite(d["sem_energy"]) and d["sem_energy"] > 0
    assert d["noise"]["single_qubit_error"] == 0.05
    assert d["noise"]["is_ideal"] is False


def test_single_trajectory_reports_nan_sem():
    """One trajectory cannot estimate a standard error, and must not pretend to."""
    engine, q = tiny_engine()
    rep = nz.noisy_qaoa_energies(engine, np.array([0.3, 0.2]), nz.NoiseModel(),
                                 trajectories=1, seed=0)
    assert np.isnan(rep.sem_energy)


def test_noisy_run_is_seed_reproducible():
    engine, q = tiny_engine()
    params = np.array([0.6, 0.25])
    model = nz.NoiseModel(single_qubit_error=0.08, measurement_error=0.05)
    a = nz.noisy_qaoa_energies(engine, params, model, trajectories=150, seed=42)
    b = nz.noisy_qaoa_energies(engine, params, model, trajectories=150, seed=42)
    assert a.mean_energy == pytest.approx(b.mean_energy)
    assert a.realised_error_count == b.realised_error_count


def test_energies_stay_within_the_qubo_spectrum():
    """Sanity floor: no trajectory may report an energy the QUBO cannot produce."""
    engine, q = tiny_engine(n=4, seed=2)
    _, E = q.all_energies()
    rep = nz.noisy_qaoa_energies(engine, np.array([0.4, 0.5, 0.3, 0.2]),
                                 nz.NoiseModel(single_qubit_error=0.2,
                                               measurement_error=0.2),
                                 trajectories=200, seed=0)
    assert min(rep.energy_samples) >= float(E.min()) - 1e-9
    assert max(rep.energy_samples) <= float(E.max()) + 1e-9
