"""Regenerate every report figure from the result CSVs.

Figures are derived artifacts, exactly like the summary tables: nothing here
computes a result, everything reads one.  Keeping them in a script rather than
in ad-hoc cells is what stops a figure from outliving the numbers it draws.

Writes figures/fig_baselines.png, fig_instance_structure.png,
fig_headtohead.png, fig_noise.png, fig_encoding_size.png.
"""
from __future__ import annotations

import sys
from pathlib import Path

import matplotlib as mpl
import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

# Three font sizes mapped to role, not to available space.
mpl.rcParams.update({
    "figure.dpi": 300, "savefig.dpi": 300,
    "font.size": 8, "axes.titlesize": 8, "axes.labelsize": 8,
    "legend.fontsize": 7, "xtick.labelsize": 6, "ytick.labelsize": 6,
    "axes.spines.top": False, "axes.spines.right": False,
    "axes.linewidth": 0.8, "xtick.major.width": 0.8, "ytick.major.width": 0.8,
    "lines.solid_capstyle": "round",
})

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
RES, FIG = ROOT / "results", ROOT / "figures"

# Colours are threaded by SOLVER across every figure: a reader who learns the
# hue on one panel never has to relearn it.
COL = {"exhaustive": "#444444", "ilp": "#444444", "tabu": "#1b7837",
       "annealing": "#4393c3", "greedy": "#d95f02",
       "qaoa_p1": "#762a83", "qaoa_p2": "#9970ab", "qaoa_p3": "#c2a5cf"}


def _finish(fig, name):
    fig.savefig(FIG / name, dpi=300, bbox_inches="tight")
    r = fig.canvas.get_renderer()
    texts = [(t, t.get_window_extent(r)) for t in fig.findobj(mpl.text.Text)
             if t.get_text().strip() and t.get_visible()]
    bad = [(a.get_text(), b.get_text()) for i, (a, ba) in enumerate(texts)
           for b, bb in texts[i + 1:] if ba.overlaps(bb)]
    plt.close(fig)
    return name, len(bad)


# ---------------------------------------------------------------- figure 1
def fig_baselines():
    """Flagship coverage curve, and where greedy stops being able to follow."""
    cv = pd.read_csv(RES / "flagship_coverage_curve.csv")
    ceiling = cv.ilp_cov.max()
    stall = cv[cv.greedy_stalled]
    k_stall = int(stall.K.min()) if len(stall) else None

    fig, (ax, ax2) = plt.subplots(1, 2, figsize=(9.2, 3.5),
                                  gridspec_kw={"width_ratios": [1.35, 1]})

    ax.axhline(ceiling, color="#999999", lw=0.8, ls=(0, (4, 3)), zorder=1)
    ax.text(2, ceiling + 0.012, "reachable ceiling", ha="left", va="bottom",
            fontsize=7, color="#666666")
    ax.plot(cv.K, cv.ilp_cov, color=COL["ilp"], lw=1.8, zorder=3)
    ax.plot(cv.K, cv.greedy_cov, color=COL["greedy"], lw=1.5, ls="--", zorder=3)
    ax.text(24, cv.loc[cv.K == 24, "ilp_cov"].iloc[0] + 0.035, "exact (ILP)",
            ha="center", va="bottom", fontsize=7, color=COL["ilp"])
    ax.text(52, cv.greedy_cov.iloc[-1] - 0.035, "greedy", ha="center", va="top",
            fontsize=7, color=COL["greedy"])
    if k_stall:
        ax.axvline(k_stall, color=COL["greedy"], lw=0.8, alpha=0.5)
        ax.annotate(f"greedy stalls at K={k_stall}\n(no improving single input)",
                    xy=(k_stall, cv.loc[cv.K == k_stall, "greedy_cov"].iloc[0]),
                    xytext=(k_stall - 3, 0.44), fontsize=7, ha="right",
                    color=COL["greedy"],
                    arrowprops=dict(arrowstyle="-", lw=0.7, color=COL["greedy"]))
    ax.set_xlabel("input budget K")
    ax.set_ylabel("fraction of 88 outcomes covered")
    ax.set_title("Exact solving is cheap; greedy hits a ceiling budget cannot lift",
                 fontsize=8.5, loc="left")
    ax.margins(0.04)

    gap = cv[cv.rel_gap.notna()]
    ax2.axhline(0, color="#bbbbbb", lw=0.8)
    ax2.vlines(gap.K, 0, gap.rel_gap * 100, color=COL["greedy"], lw=1.4)
    ax2.plot(gap.K, gap.rel_gap * 100, "o", ms=2.6, color=COL["greedy"])
    ax2.set_xlabel("input budget K")
    ax2.set_ylabel("greedy shortfall vs certified\noptimum (% of optimum)")
    ax2.set_title("Shortfall appears at large budgets, not tight ones",
                  fontsize=8.5, loc="left")
    ax2.margins(0.04)
    fig.tight_layout()
    return _finish(fig, "fig_baselines.png")


# ---------------------------------------------------------------- figure 2
def fig_instance_structure():
    """Which structural regimes make greedy fail at all."""
    dp = pd.read_csv(RES / "discriminating_power.csv")
    rate = (1 - dp.groupby("regime").greedy_optimal.mean()).sort_values()
    n = dp.groupby("regime").size()

    fig, ax = plt.subplots(figsize=(5.6, 3.2))
    y = np.arange(len(rate))
    colors = ["#999999" if r == "profile" else COL["greedy"] for r in rate.index]
    ax.hlines(y, 0, rate.values * 100, color=colors, lw=1.2)
    ax.plot(rate.values * 100, y, "o", ms=5,
            color="none", markeredgecolor="none")
    for yi, (name, v), col in zip(y, rate.items(), colors):
        ax.plot(v * 100, yi, "o", ms=5, color=col)
        ax.text(v * 100 + 1.2, yi, f"{v*100:.0f}%", va="center", fontsize=7,
                color=col)
    ax.set_yticks(y)
    ax.set_yticklabels([("real profile" if r == "profile" else r.replace("_", " "))
                        for r in rate.index])
    ax.set_xlabel("cells where greedy misses the certified optimum (%)")
    ax.set_title("Only some structures separate the optimizers",
                 fontsize=8.5, loc="left")
    # The profile row reads 0% because this survey sweeps only tight budgets.
    # Greedy's failures on the flagship all sit above its stall (Fig. baselines),
    # outside the fractions swept here; state the range so 0% is not read as
    # "the real instance never separates".
    fr = sorted(dp.K_frac.unique())
    ax.text(0.0, -0.30, f"n = {int(n.sum())} cells (regime \u00d7 size \u00d7 budget "
            f"\u00d7 seed); higher = more informative",
            transform=ax.transAxes, ha="left", fontsize=6.5, color="#666666")
    ax.text(0.0, -0.40, f"budgets swept: K/n = {fr[0]:.2f}\u2013{fr[-1]:.2f}; "
            "the flagship's greedy failures lie above this range",
            transform=ax.transAxes, ha="left", fontsize=6.5, color="#666666")
    ax.margins(0.06)
    ax.set_xlim(left=0)
    fig.tight_layout()
    return _finish(fig, "fig_instance_structure.png")


# ---------------------------------------------------------------- figure 3
def fig_headtohead():
    """Every solver on the identical QUBO: hit rate and gap."""
    hh = pd.read_csv(RES / "headtohead.csv")
    order = (hh.groupby("algorithm").found_optimum.mean()
             .sort_values(ascending=False).index.tolist())

    fig, (ax, ax2) = plt.subplots(1, 2, figsize=(8.8, 3.3))
    for i, alg in enumerate(order):
        g = hh[hh.algorithm == alg]
        ax.bar(i, g.found_optimum.mean() * 100, width=0.62,
               color=COL.get(alg, "#888888"),
               edgecolor="none")
        ax.text(i, g.found_optimum.mean() * 100 + 1.5,
                f"{g.found_optimum.mean()*100:.0f}", ha="center", fontsize=7,
                color=COL.get(alg, "#888888"))
    ax.set_xticks(range(len(order)))
    ax.set_xticklabels([a.replace("qaoa_p", "QAOA\np=") for a in order])
    ax.set_ylabel("runs reaching the QUBO ground state (%)")
    ax.set_title("Classical solvers reach the optimum more often",
                 fontsize=8.5, loc="left")
    ax.set_ylim(0, 112)

    for i, alg in enumerate(order):
        g = hh[hh.algorithm == alg]
        x = np.random.default_rng(0).normal(i, 0.07, len(g))
        ax2.plot(x, g.gap, "o", ms=3, alpha=0.55,
                 color=COL.get(alg, "#888888"), mec="none")
        ax2.hlines(g.gap.median(), i - 0.28, i + 0.28, lw=1.6,
                   color=COL.get(alg, "#888888"))
    ax2.set_xticks(range(len(order)))
    ax2.set_xticklabels([a.replace("qaoa_p", "QAOA\np=") for a in order])
    ax2.set_ylabel("energy above ground state")
    ax2.set_title("Gaps are small everywhere: 25 instances, 8\u201316 inputs",
                  fontsize=8.5, loc="left")
    ax2.set_ylim(bottom=-0.12)
    ax2.text(0.0, -0.20, "lower = better", transform=ax2.transAxes,
             ha="left", fontsize=6.5, color="#666666")
    fig.tight_layout()
    return _finish(fig, "fig_headtohead.png")


# ---------------------------------------------------------------- figure 4
def fig_noise():
    """Degradation by mechanism, with the resolution floor made visible."""
    ns = pd.read_csv(RES / "noise_sweep.csv")
    d = ns[ns.rate > 0]
    mech_col = {"two_qubit_error": "#762a83", "single_qubit_error": "#4393c3",
                "measurement_error": "#7fbf7b"}

    fig, axes = plt.subplots(1, 2, figsize=(8.8, 3.3), sharey=True)
    for ax, chan in zip(axes, ["depolarizing", "dephasing"]):
        for mech, col in mech_col.items():
            g = (d[(d.channel == chan) & (d.mechanism == mech)]
                 .groupby("rate").agg(dE=("delta_E", "mean"),
                                      se=("delta_E", "sem")))
            ax.errorbar(g.index * 100, g.dE, yerr=g.se, color=col, lw=1.4,
                        marker="o", ms=3.5, capsize=2)
        ax.axhline(0, color="#bbbbbb", lw=0.8)
        ax.set_xlabel("error rate (%)")
        ax.set_title(f"{chan} channel", fontsize=8.5, loc="left")
        ax.margins(0.05)
    axes[0].set_ylabel("mean sampled energy above ideal")
    for mech, col in mech_col.items():
        axes[1].plot([], [], color=col, marker="o", ms=3.5, lw=1.4,
                     label=mech.replace("_", " "))
    axes[1].legend(frameon=False, fontsize=7, loc="upper left")
    fig.suptitle("Two-qubit gate error dominates; readout error is nearly free",
                 fontsize=9, x=0.008, ha="left")
    fig.tight_layout(rect=(0, 0, 1, 0.94))
    return _finish(fig, "fig_noise.png")


# ---------------------------------------------------------------- figure 5
def fig_encoding_size():
    """Qubit cost of each exact encoding against inputs."""
    ev = pd.read_csv(RES / "encoding_var_counts.csv")
    fig, ax = plt.subplots(figsize=(5.4, 3.2))
    palette = {"pairwise": "#1b7837", "slack": "#762a83", "penalty": "#4393c3",
               "unary": "#d95f02"}
    for enc, g in ev.groupby("enc"):
        g = g.sort_values("n_inputs")
        ax.plot(g.n_inputs, g.n_vars, marker="o", ms=3.5, lw=1.5,
                color=palette.get(enc, "#888888"))
        ax.text(g.n_inputs.iloc[-1] + 0.3, g.n_vars.iloc[-1], enc, fontsize=7.5,
                va="center", color=palette.get(enc, "#888888"))
    ax.axhline(30, color="#cc3311", lw=0.9, ls=(0, (4, 3)))
    ax.text(ev.n_inputs.max(), 34, "state-vector limit on 48 GiB",
            fontsize=6.8, color="#cc3311", va="bottom", ha="right")
    ax.set_xlabel("biomedical inputs")
    ax.set_ylabel("QUBO variables (= qubits)")
    ax.set_title("Inputs are not qubits: encoding sets the simulable range",
                 fontsize=8.5, loc="left")
    ax.margins(0.1)
    ax.set_xlim(right=ev.n_inputs.max() * 1.16)
    fig.tight_layout()
    return _finish(fig, "fig_encoding_size.png")


def main() -> None:
    FIG.mkdir(exist_ok=True)
    for fn in (fig_baselines, fig_instance_structure, fig_headtohead,
               fig_noise, fig_encoding_size):
        name, overlaps = fn()
        print(f"{name:32s} text overlaps: {overlaps}")


if __name__ == "__main__":
    main()
