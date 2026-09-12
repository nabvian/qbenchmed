"""Noise models for QAOA robustness analysis (spec1 sections 47-48; spec3 s12).

Method, stated plainly because it bounds what these numbers mean: noise is
applied by *trajectory sampling* on the state vector, not by density-matrix
evolution.  Each shot follows one stochastic realisation of the noise channel,
and the reported distribution is the average over trajectories.  This is exact
in the limit of many trajectories and is memory-feasible at the qubit counts the
project reaches; a density-matrix simulation would square the memory cost and
halve the reachable width, which is the wrong trade for a benchmark whose whole
purpose is to compare across sizes.

The consequence is that trajectory noise carries its own sampling error, so a
noisy result is never compared against an ideal result without also reporting
the trajectory count.  Ideal simulation, noisy simulation and hardware execution
are three separate categories and are never conflated (spec3 s12); nothing in
this module claims to reproduce any specific device.
"""

from __future__ import annotations

from dataclasses import dataclass, field

import numpy as np

from .simulator import StateVector

#: Channels implemented.  Named after the physical mechanism, not a vendor.
CHANNELS = ("depolarizing", "dephasing", "amplitude_damping", "bit_flip")


@dataclass
class NoiseModel:
    """Per-operation error rates.

    All rates default to zero, so ``NoiseModel()`` is the ideal case and an
    experiment can sweep one channel while provably leaving the others off.
    """

    single_qubit_error: float = 0.0
    two_qubit_error: float = 0.0
    measurement_error: float = 0.0
    channel: str = "depolarizing"
    #: Idle/decoherence error applied once per QAOA layer, per qubit.
    layer_idle_error: float = 0.0

    def __post_init__(self):
        if self.channel not in CHANNELS:
            raise ValueError(f"unknown channel {self.channel!r}; use one of {CHANNELS}")
        for name in ("single_qubit_error", "two_qubit_error", "measurement_error",
                     "layer_idle_error"):
            v = getattr(self, name)
            if not (0.0 <= v <= 1.0):
                raise ValueError(f"{name}={v} outside [0, 1]")

    @property
    def is_ideal(self) -> bool:
        return (self.single_qubit_error == 0.0 and self.two_qubit_error == 0.0
                and self.measurement_error == 0.0 and self.layer_idle_error == 0.0)

    def as_dict(self) -> dict:
        return {**self.__dict__, "is_ideal": self.is_ideal}


# Pauli matrices used by the channels.
_X = np.array([[0, 1], [1, 0]], dtype=np.complex128)
_Y = np.array([[0, -1j], [1j, 0]], dtype=np.complex128)
_Z = np.array([[1, 0], [0, -1]], dtype=np.complex128)


def apply_channel(sv: StateVector, qubit: int, rate: float, channel: str,
                  rng: np.random.Generator) -> bool:
    """Apply one stochastic realisation of ``channel`` to ``qubit``.

    Returns whether an error was injected, so an experiment can report the
    realised error count rather than only the nominal rate -- at small trajectory
    counts these differ, and the realised number is the honest one.

    The uniform draw is taken *unconditionally*, including at ``rate == 0``.
    Short-circuiting on a zero rate would consume a different number of random
    numbers than a noisy run, so the ideal baseline would be drawn from a
    different random stream than the points it is compared against -- turning a
    pure rng offset into an apparent noise effect.  Paying one wasted draw per
    call buys a genuinely paired zero-noise reference.
    """
    draw = rng.random()
    if rate <= 0.0 or draw >= rate:
        return False

    if channel == "depolarizing":
        sv.apply_1q(rng.choice([_X, _Y, _Z]), qubit, count=False)
    elif channel == "dephasing":
        sv.apply_1q(_Z, qubit, count=False)
    elif channel == "bit_flip":
        sv.apply_1q(_X, qubit, count=False)
    elif channel == "amplitude_damping":
        # trajectory form: project toward |0> on the damped qubit, then
        # renormalise.  Norm loss here is physical, not numerical drift.
        proj = np.zeros((2, 2), dtype=np.complex128)
        proj[0, 0] = 1.0
        sv.apply_1q(proj, qubit, count=False)
        nrm = sv.norm()
        if nrm > 1e-12:
            sv.psi /= nrm
        else:                       # fully damped: reset to |0...0>
            sv.psi[...] = 0.0
            sv.psi.reshape(-1)[0] = 1.0
    else:                                                # pragma: no cover
        raise ValueError(f"unknown channel {channel!r}")
    return True


@dataclass
class NoisyRunReport:
    """Result of a noisy QAOA evaluation, with its own uncertainty attached."""

    trajectories: int
    mean_energy: float
    sem_energy: float
    best_energy: float
    optimum_probability: float
    realised_error_count: int
    noise: dict
    energy_samples: list[float] = field(default_factory=list)
    #: The lowest-energy bitstring drawn across trajectories.  Kept so a noisy
    #: point can be reported as an *answer* -- a selection with a coverage and a
    #: feasibility status -- and not only as a distribution of energies.
    best_state: list[int] = field(default_factory=list)

    def as_dict(self) -> dict:
        d = dict(self.__dict__)
        d.pop("energy_samples")
        return d


def noisy_qaoa_energies(engine, params: np.ndarray, noise: NoiseModel, *,
                        trajectories: int = 200, seed: int | None = None,
                        optimum_energy: float | None = None) -> NoisyRunReport:
    """Evaluate a QAOA circuit under trajectory noise.

    ``engine`` is a :class:`~qbm.qaoa.Qaoa`.  Noise is injected after each cost
    layer (as a two-qubit-gate proxy scaled by the coupling count), after each
    mixer rotation, once per layer as idle error, and at measurement -- which is
    where a real device accumulates it.  One bitstring is drawn per trajectory,
    matching how a device would return one shot per circuit execution.
    """
    rng = np.random.default_rng(seed)
    p = len(params) // 2
    gammas, betas = params[:p], params[p:]
    n = engine.n
    phase_shape = (2,) * n
    # The cost phase MUST be rebuilt per gamma as exp(-i*gamma*E), never as
    # exp(-i*E)**gamma: the latter wraps E into the principal branch (-pi, pi]
    # *before* scaling, so any |E| > pi gets the wrong phase.  With penalty
    # terms pushing the spectrum to ~1e3, that is almost every state, and it
    # silently applied a different cost unitary than the ideal path.
    diag = engine.diag.reshape(phase_shape)

    # a two-qubit error per coupling is the honest proxy for a cost layer whose
    # exact simulation skips the gate sequence entirely
    n_couplings = int(np.count_nonzero(np.triu(engine.J, 1)))

    energies = np.empty(trajectories)
    best_bits = None
    best_e = np.inf
    errors = 0
    for t in range(trajectories):
        sv = StateVector.plus_state(n, memory_limit_bytes=engine.memory_limit_bytes)
        for gamma, beta in zip(gammas, betas):
            sv.psi = sv.psi * np.exp(-1j * gamma * diag)
            # cost layer: two-qubit error budget spread over the couplings
            if noise.two_qubit_error > 0.0 and n_couplings:
                per_qubit = 1.0 - (1.0 - noise.two_qubit_error) ** (
                    2 * n_couplings / max(n, 1))
                for qb_ in range(n):
                    errors += apply_channel(sv, qb_, per_qubit, noise.channel, rng)
            sv.rx_all(float(beta))
            for qb_ in range(n):
                errors += apply_channel(sv, qb_, noise.single_qubit_error,
                                        noise.channel, rng)
                errors += apply_channel(sv, qb_, noise.layer_idle_error,
                                        noise.channel, rng)

        state, _ = sv.measure(shots=1, seed=int(rng.integers(2 ** 31)))
        bits = state[0].astype(bool).copy()
        if noise.measurement_error > 0.0:
            flip = rng.random(n) < noise.measurement_error
            errors += int(flip.sum())
            bits ^= flip
        energies[t] = engine.qubo.energy(bits)
        if energies[t] < best_e:
            best_e, best_bits = float(energies[t]), bits.copy()

    ref = float(optimum_energy) if optimum_energy is not None else float(energies.min())
    return NoisyRunReport(
        trajectories=trajectories,
        mean_energy=float(energies.mean()),
        sem_energy=float(energies.std(ddof=1) / np.sqrt(trajectories))
        if trajectories > 1 else float("nan"),
        best_energy=float(energies.min()),
        optimum_probability=float(np.mean(energies <= ref + 1e-9)),
        realised_error_count=int(errors),
        noise=noise.as_dict(),
        energy_samples=[float(v) for v in energies],
        best_state=[int(b) for b in best_bits] if best_bits is not None else [])
