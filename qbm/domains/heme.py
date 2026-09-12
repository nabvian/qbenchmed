"""Q-BenchMed-Heme: the 66-input / 88-outcome hematology domain adapter.

This module is the *only* place in the repository that contains biomedical
content.  It builds a `qbm.profile.DomainProfile` from the PATHEX audit export
and nothing else; the optimization core never imports it (spec section 24).

Provenance
----------
Both the outcome trigger table and the input wiring audit are transcribed from
a single source document:

    PATHEX -- 88 Outcome x Input Trigger and Wiring Matrix
    audit snapshot 2026-08-29
    runtime source of truth: PATHEX/PRO-EXEC, bundle_version 1.2.0,
    executable_status EXECUTABLE

That document is an *implementation audit* of a running rule engine, not a
clinical specification.  Its own preamble is explicit that documenting a
condition does not mean the condition is clinically correct, and Q-BenchMed
inherits that caveat unchanged: the edges below describe which inputs the
engine currently reads, not which inputs a clinician ought to order.

Every edge here is traceable to a numbered row of that table.  Nothing is
invented.  Where the audit records that a trigger cannot be encoded from the
declared 66 inputs, the arm is dropped and listed in `DROPPED_ARMS` rather than
guessed at.

Encoding rules
--------------
The audit states triggers in prose.  Four rules turn that prose into edges, and
they are applied uniformly:

1.  **Only positive conditions create dependencies.**  A negated guard --
    "not the tri-lineage combination", "no pancytopenia", "B12 not low",
    "ABC not >0.1" -- is *satisfied when its input is absent*, so it creates no
    dependency for a panel-selection problem.  The audit confirms this reading
    explicitly for outcome 77: "A classified HGB <8 blocks it; missing HGB does
    not create that flag."  A condition that requires an input to be supplied
    with a particular value (outcome 21, "DAT negative") is positive, because
    the guard fails when DAT is missing.

2.  **Engine-derived values expand to the raw inputs they read.**  The audit
    lists 11 computed values that are not user inputs; `DERIVED` records the
    expansion used for each.  So a Mentzer condition becomes {MCV, RBC}, an ANC
    condition becomes {WBC, NEUT}, and so on.

3.  **Patient context and UI-only Boolean fields are ambient.**  Age, sex,
    pregnancy, fever, splenomegaly, endemic-area context, heparin exposure and
    the other context flags sit outside the 66 (the audit says so in its own
    section 2) and are not orderable tests.  A panel-selection optimizer cannot
    choose whether the patient has a fever, so these fields are treated as
    always available at zero cost and create no decision variables.  Any arm
    whose only non-CBC driver is ambient is therefore satisfiable from its
    laboratory inputs alone.

4.  **A disjunction becomes separate arms; a conjunction becomes one arm.**
    "RDW-CV >14.5 or RDW-SD >46" yields two arms.  "MCV <80 + Mentzer >14 +
    RDW elevated + RBC normal/low + MCH <27" yields one arm containing all of
    the inputs those conditions read.

Because the resolver stops at the first matching rule, a lower-priority rule can
be unreachable in practice.  The audit's wiring column records that as
"Shadowed"; those outcomes keep their authored arms here, with the shadowing
noted in `OUTCOME_WIRING`, because the optimization question ("can a panel
supply the inputs this rule reads?") is well posed whether or not another rule
wins first.  Consumers that want reachable-only behaviour should filter on the
wiring status rather than on the edge set.

Self-validation
---------------
The audit reports, independently of the trigger table, that 42 of the 66
declared inputs currently drive an outcome and 24 do not.  `build_profile()`
asserts that the edge set derived from the trigger table reproduces exactly
that 42/24 split and exactly the audit's per-input verdicts.  The encoding is
therefore checked against a number that was not used to produce it: a
transcription error in any row shows up as a mismatch rather than passing
silently.

Costs and weights
-----------------
Both are uniform.  Spec section 9.3 requires that costs be explicitly defined
and justified rather than invented after seeing results, and this export carries
no cost information -- no prices, no turnaround times, no panel groupings with
marginal costs.  Inventing a cost vector here would make every
cost-constrained result an artifact of that invention.  `ACQUISITION_TIER`
records each input's ordering group as *metadata only*, so that a future
justified cost model can use it; the default objective does not read it.
Outcome weights are uniform, matching the `outcome_weighting: uniform` default
of spec section 27.
"""

from __future__ import annotations

from qbm.profile import DomainProfile, Relationship

SOURCE_DOC = (
    "PATHEX 88 Outcome x Input Trigger and Wiring Matrix, audit snapshot "
    "2026-08-29, PRO-EXEC bundle_version 1.2.0"
)
KNOWLEDGE_VERSION = "pathex-audit-2026-08-29"
BENCHMARK_ID = "QBMED-HEME-001"

# --------------------------------------------------------------------------
# The 66 declared inputs (audit section 2 and section 7).
#
# The count reconstructs as 16 raw CBC + 36 optional external laboratory + 13
# coded smear findings + BONE_PAIN.  CALCIUM is duplicated between the
# first-class and external registries in the source system and is counted once.
# --------------------------------------------------------------------------

CBC_INPUTS = (
    "HGB", "RBC", "HCT", "MCV", "MCH", "MCHC", "RDW_CV", "RDW_SD",
    "WBC", "PLT", "MPV", "NEUT", "LYMPH", "MONO", "EOS", "BASO",
)

EXTERNAL_INPUTS = (
    "FERRITIN", "TSAT", "B12", "FOLATE", "RETIC", "HBA2", "ESR", "CRP",
    "CREATININE", "eGFR", "CALCIUM", "POTASSIUM", "SODIUM", "UREA",
    "BILIRUBIN_TOTAL", "BILIRUBIN_INDIRECT", "AST", "ALT", "ALBUMIN",
    "TSH", "FT4", "PT_PROLONGATION_SEC", "FIBRINOGEN", "D_DIMER", "APTT",
    "LDH", "HAPTOGLOBIN", "MMA", "HOMOCYSTEINE", "TIBC", "STFR", "HBF",
    "SERUM_COPPER", "CERULOPLASMIN", "DAT_COOMBS", "G6PD_ASSAY",
)

SMEAR_INPUTS = (
    "target_cells", "basophilic_stippling", "pencil_cigar_cells",
    "spherocytes", "schistocytes", "teardrop_dacrocytes",
    "howell_jolly_bodies", "hypersegmented_neutrophils", "bite_cells",
    "sickle_cells", "rouleaux", "nucleated_rbc", "blasts_auer_rods",
)

FIRST_CLASS_INPUTS = ("BONE_PAIN",)

INPUT_IDS = CBC_INPUTS + EXTERNAL_INPUTS + SMEAR_INPUTS + FIRST_CLASS_INPUTS

ACQUISITION_TIER = {
    **{i: "cbc" for i in CBC_INPUTS},
    **{i: "external_lab" for i in EXTERNAL_INPUTS},
    **{i: "smear" for i in SMEAR_INPUTS},
    **{i: "history" for i in FIRST_CLASS_INPUTS},
}

# Engine-computed values (audit section 2) and the raw inputs each reads
# (audit section 3).  Not user inputs; never decision variables.
DERIVED = {
    "MENTZER": ("MCV", "RBC"),
    "PATRA": ("MCV", "RDW_CV"),
    "RDWI_NUMERATOR": ("MCV", "RDW_CV", "RBC"),
    "RDWI": ("MCV", "RDW_CV", "RBC"),
    "CORRECTED_RETIC": ("RETIC", "HCT"),
    "LDH_FOLD_ULN": ("LDH",),          # also needs LDH_ULN, absent from the 66
    "ANC": ("WBC", "NEUT"),
    "ALC": ("WBC", "LYMPH"),
    "AMC": ("WBC", "MONO"),
    "AEC": ("WBC", "EOS"),
    "ABC": ("WBC", "BASO"),
}

# Context and UI-only fields the rules read that sit outside the 66
# (audit section 2).  Ambient: always available, zero cost, not variables.
AMBIENT_CONTEXT = (
    "age", "sex", "pregnancy", "fever", "splenomegaly", "endemic_area",
    "heparin_use", "capillary_sample", "lead_or_arsenic_exposure",
    "mds_context", "tb_context", "hiv_risk", "dic_predisposing_illness",
    "endemic_mycosis_exposure", "barefoot_rural", "family_history_low_platelets",
    "constitutional_neutropenia_declared", "context_inflammation",
    "context_bone_pain",
)

# --------------------------------------------------------------------------
# The 88 declared outcomes, in audit row order, with the audit's wiring status.
# --------------------------------------------------------------------------

OUTCOMES: tuple[tuple[int, str, str], ...] = (
    (1, "IDA", "UI"),
    (2, "BTT", "UI"),
    (3, "ALPHA_TRAIT_SUSPECT", "UI/review"),
    (4, "DIMORPHIC", "UI"),
    (5, "IRON_REPLETE_MICROCYTOSIS", "UI/review"),
    (6, "SIDEROBLASTIC_SUSPECT", "UI/review"),
    (7, "ACD_SUSPECT", "UI"),
    (8, "NORMAL_CBC", "deprecated"),
    (9, "CANNOT_RESOLVE_ON_CBC", "UI"),
    (10, "CRITICAL_THROMBOCYTOPENIA", "UI"),
    (11, "COEXISTENCE_BTT_IDA", "UI/review"),
    (12, "URGENT_HAEMATOLOGY_REVIEW", "UI"),
    (13, "MACROCYTIC_ANEMIA", "UI/review"),
    (14, "NORMOCYTIC_ANEMIA", "UI/review"),
    (15, "THALASSAEMIA_TRAIT_SUSPECT", "deprecated"),
    (16, "HBPATHY_SUSPECT", "UI"),
    (17, "SICKLE_SUSPECT", "UI"),
    (18, "HAEMOLYSIS_SUSPECT", "api_only"),
    (19, "COMPENSATED_HAEMOLYSIS_SUSPECT", "api_only"),
    (20, "AIHA_SUSPECT", "api_only"),
    (21, "PNH_SUSPECT", "api_only"),
    (22, "MEMBRANOPATHY_SUSPECT", "UI/review"),
    (23, "G6PD_DEFICIENCY_SUSPECT", "UI"),
    (24, "BLOOD_LOSS_SUSPECT", "UI/review"),
    (25, "HYPOPROLIFERATIVE_SUSPECT", "UI/review"),
    (26, "IRON_DEFICIENCY_NO_ANAEMIA", "UI"),
    (27, "DUAL_DEFICIENCY_SUSPECT", "UI/review"),
    (28, "LEAD_TOXICITY_SUSPECT", "UI/review"),
    (29, "COPPER_DEFICIENCY_SUSPECT", "api_only"),
    (30, "ERYTHROCYTOSIS_SUSPECT", "UI/review"),
    (31, "MEGALOBLASTIC_SUSPECT", "UI"),
    (32, "B12_DEFICIENCY_SUSPECT", "UI"),
    (33, "FOLATE_DEFICIENCY_SUSPECT", "UI"),
    (34, "NON_MEGALOBLASTIC_MACROCYTOSIS_SUSPECT", "UI/review"),
    (35, "MICROCYTIC_ANAEMIA_UNTYPED", "UI"),
    (36, "MICROCYTIC_BORDERLINE_INDICES", "UI"),
    (37, "MICROCYTIC_BORDERLINE_PATRA_LEANS_TRAIT", "UI"),
    (38, "MICROCYTIC_BORDERLINE_PATRA_LEANS_IRON_DEFICIENCY", "UI"),
    (39, "HBA2_EQUIVOCAL_BAND", "UI/review"),
    (40, "HBA2_VARIANT_RANGE", "UI"),
    (41, "MDS_SUSPECT", "UI/review"),
    (42, "ACUTE_LEUKAEMIA_SUSPECT", "UI"),
    (43, "CLL_CLPD_SUSPECT", "UI/review"),
    (44, "CML_SUSPECT", "UI"),
    (45, "MPN_SUSPECT", "UI"),
    (46, "MYELOMA_SUSPECT", "UI/review"),
    (47, "APLASTIC_ANAEMIA_SUSPECT", "UI"),
    (48, "MARROW_PROCESS_SUSPECT", "UI"),
    (49, "MARROW_INFILTRATION_SUSPECT", "UI"),
    (50, "PROLYMPHOCYTIC_ATYPICAL_LYMPHOID_SUSPECT", "UI/review"),
    (51, "PANCYTOPENIA_SUSPECT", "UI"),
    (52, "HLH_SUSPECT", "UI/review"),
    (53, "NEUTROPENIA_SUSPECT", "UI/review"),
    (54, "AGRANULOCYTOSIS_SUSPECT", "UI/review"),
    (55, "BENIGN_ETHNIC_NEUTROPENIA_CONTEXT", "UI"),
    (56, "LYMPHOPENIA_SUSPECT", "UI"),
    (57, "EOSINOPHILIA_SUSPECT", "UI"),
    (58, "EOSINOPHILIA_INVESTIGATE", "UI"),
    (59, "HYPEREOSINOPHILIA_SUSPECT", "UI"),
    (60, "LEUKAEMOID_REACTION_SUSPECT", "UI"),
    (61, "THROMBOCYTOPENIA_SUSPECT", "UI"),
    (62, "INHERITED_THROMBOCYTOPENIA_SUSPECT", "UI/major_review"),
    (63, "HIT_SUSPECT", "UI/review"),
    (64, "TMA_SUSPECT", "UI"),
    (65, "DIC_SUSPECT", "UI/review"),
    (66, "REACTIVE_THROMBOCYTOSIS_SUSPECT", "UI/review"),
    (67, "MALARIA_SUSPECT", "UI"),
    (68, "DENGUE_SUSPECT", "shadowed"),
    (69, "KALA_AZAR_SUSPECT", "UI"),
    (70, "TB_ANAEMIA_SUSPECT", "UI"),
    (71, "HOOKWORM_STH_IDA_SUSPECT", "UI/review"),
    (72, "ENTERIC_FEVER_CONTEXT", "UI"),
    (73, "SEPSIS_CONTEXT", "UI/review"),
    (74, "VIRAL_PATTERN_CONTEXT", "UI/review"),
    (75, "HIV_CONTEXT_FLAG", "UI"),
    (76, "FUNGAL_MARROW_SUSPECT", "UI"),
    (77, "HYPERSPLENISM_CONTEXT", "UI"),
    (78, "HYPERTHYROID_ANAEMIA_SUSPECT", "api_only"),
    (79, "THYROID_RESULT_NEEDS_ACTION", "UI"),
    (80, "CAPILLARY_SCREEN_CONFIRM_VENOUS", "UI"),
    (81, "SEVERE_ANEMIA_SAFETY_BLOCK", "UI"),
    (82, "CANNOT_EVALUATE", "UI"),
    (83, "NON_MICROCYTIC_ANEMIA_OUTSIDE_SCOPE", "shadowed"),
    (84, "ANEMIA_UNCLASSIFIED", "fallback/review"),
    (85, "NO_FINDINGS", "UI"),
    (86, "IDA_PROBABLE", "UI/review"),
    (87, "THALASSEMIA_TRAIT_PROBABLE", "UI"),
    (88, "THALASSEMIA_TRAIT_POSSIBLE", "UI/review"),
)

OUTCOME_IDS = tuple(o[1] for o in OUTCOMES)
OUTCOME_ROW = {o[1]: o[0] for o in OUTCOMES}
OUTCOME_WIRING = {o[1]: o[2] for o in OUTCOMES}

# --------------------------------------------------------------------------
# Rule arms.  outcome_id -> ((arm_label, (input_id, ...)), ...)
#
# Inputs within an arm are conjunctive; arms are disjunctive.  An outcome with
# no arms is unreachable from the declared 66 inputs -- either deprecated, or
# driven only by a field outside the registry (see DROPPED_ARMS).
# --------------------------------------------------------------------------

ARMS: dict[str, tuple[tuple[str, tuple[str, ...]], ...]] = {
    # --- core outcomes 1-14 ------------------------------------------------
    # 1: HGB anaemic + MCV <=100 + confirmed iron deficiency.  Iron deficiency
    # confirms via ferritin <15, or ferritin <70 with inflammation context
    # (ambient) or CRP >5.  Arm A covers the first two; arm B the CRP route.
    "IDA": (("ferritin", ("HGB", "MCV", "FERRITIN")),
            ("ferritin_crp", ("HGB", "MCV", "FERRITIN", "CRP"))),
    # 2: "None required beyond an MCV class" -- an MCV class must exist, but
    # either side of 80 satisfies it.
    "BTT": (("hba2", ("MCV", "HBA2")),),
    "ALPHA_TRAIT_SUSPECT": (("hba2", ("MCV", "HBA2")),),
    # 4: two materially different routes share one label (audit note).
    "DIMORPHIC": (("macro_iron", ("HGB", "MCV", "FERRITIN")),
                  ("macro_iron_crp", ("HGB", "MCV", "FERRITIN", "CRP")),
                  ("normo_rdwsd", ("HGB", "MCV", "RDW_SD")),
                  ("normo_rdwcv", ("HGB", "MCV", "RDW_CV"))),
    "IRON_REPLETE_MICROCYTOSIS": (("ferritin", ("MCV", "FERRITIN")),
                                  ("tsat", ("MCV", "TSAT"))),
    "SIDEROBLASTIC_SUSPECT": (("iron_loaded", ("FERRITIN", "TSAT")),),
    "ACD_SUSPECT": (("masked_id", ("FERRITIN", "TSAT")),),
    "NORMAL_CBC": (),
    # 9: path A is any of AMC/ABC/ALC high or WBC >50 -- four arms.
    "CANNOT_RESOLVE_ON_CBC": (("amc", ("WBC", "MONO")),
                              ("abc", ("WBC", "BASO")),
                              ("alc", ("WBC", "LYMPH")),
                              ("wbc_extreme", ("WBC",)),
                              ("bicytopenia", ("MCV", "WBC", "PLT"))),
    "CRITICAL_THROMBOCYTOPENIA": (("plt", ("PLT",)),),
    "COEXISTENCE_BTT_IDA": (("mentzer_iron", ("MCV", "RBC", "FERRITIN")),
                            ("mentzer_iron_crp",
                             ("MCV", "RBC", "FERRITIN", "CRP")),
                            ("cbc_only", ("MCV", "RBC", "RDW_CV"))),
    "URGENT_HAEMATOLOGY_REVIEW": (("blasts", ("blasts_auer_rods",)),),
    "MACROCYTIC_ANEMIA": (("macro", ("MCV", "HGB")),),
    "NORMOCYTIC_ANEMIA": (("normo", ("MCV", "HGB")),),

    # --- red-cell and anaemia mechanisms 15-30 -----------------------------
    "THALASSAEMIA_TRAIT_SUSPECT": (),
    "HBPATHY_SUSPECT": (("target", ("MCV", "target_cells")),),
    "SICKLE_SUSPECT": (("sickle", ("sickle_cells",)),),
    # 18: three authored smear arms plus the effective haptoglobin arm.
    "HAEMOLYSIS_SUSPECT": (("spherocytes", ("HGB", "spherocytes")),
                           ("bite_cells", ("HGB", "bite_cells")),
                           ("sickle_cells", ("HGB", "sickle_cells")),
                           ("haptoglobin", ("HGB", "HAPTOGLOBIN"))),
    "COMPENSATED_HAEMOLYSIS_SUSPECT": (("haptoglobin", ("HGB", "HAPTOGLOBIN")),),
    "AIHA_SUSPECT": (("dat_pos", ("HGB", "HAPTOGLOBIN", "DAT_COOMBS")),),
    "PNH_SUSPECT": (("dat_neg", ("HGB", "HAPTOGLOBIN", "DAT_COOMBS")),),
    "MEMBRANOPATHY_SUSPECT": (("spherocytes", ("HGB", "spherocytes")),),
    "G6PD_DEFICIENCY_SUSPECT": (("bite_cells", ("HGB", "bite_cells")),),
    "BLOOD_LOSS_SUSPECT": (("retic_high", ("HGB", "MCV", "RETIC")),),
    "HYPOPROLIFERATIVE_SUSPECT": (("retic_low", ("HGB", "MCV", "RETIC")),),
    "IRON_DEFICIENCY_NO_ANAEMIA": (("ferritin", ("HGB", "FERRITIN")),
                                   ("ferritin_crp", ("HGB", "FERRITIN", "CRP"))),
    "DUAL_DEFICIENCY_SUSPECT": (("iron_b12", ("HGB", "FERRITIN", "B12")),
                                ("iron_crp_b12",
                                 ("HGB", "FERRITIN", "CRP", "B12"))),
    "LEAD_TOXICITY_SUSPECT": (("stippling", ("basophilic_stippling",)),),
    "COPPER_DEFICIENCY_SUSPECT": (
        ("copper", ("HGB", "SERUM_COPPER", "CERULOPLASMIN")),),
    "ERYTHROCYTOSIS_SUSPECT": (("hgb_high", ("HGB",)),),

    # --- macrocytic and haematinic 31-34 -----------------------------------
    "MEGALOBLASTIC_SUSPECT": (
        ("hyperseg", ("MCV", "HGB", "hypersegmented_neutrophils")),),
    "B12_DEFICIENCY_SUSPECT": (("generic", ("B12",)),
                               ("pancytopenic", ("HGB", "WBC", "PLT", "B12"))),
    "FOLATE_DEFICIENCY_SUSPECT": (
        ("generic", ("FOLATE",)),
        ("pancytopenic", ("HGB", "WBC", "PLT", "FOLATE"))),
    "NON_MEGALOBLASTIC_MACROCYTOSIS_SUSPECT": (
        ("hypothyroid", ("MCV", "HGB", "TSH")),),

    # --- microcytic discrimination 35-40 -----------------------------------
    "MICROCYTIC_ANAEMIA_UNTYPED": (("fallback", ("HGB", "MCV")),),
    "MICROCYTIC_BORDERLINE_INDICES": (("mentzer_only", ("MCV", "RBC")),),
    "MICROCYTIC_BORDERLINE_PATRA_LEANS_TRAIT": (
        ("patra", ("MCV", "RBC", "RDW_CV")),),
    "MICROCYTIC_BORDERLINE_PATRA_LEANS_IRON_DEFICIENCY": (
        ("patra", ("MCV", "RBC", "RDW_CV")),),
    "HBA2_EQUIVOCAL_BAND": (("hba2", ("HBA2",)),),
    "HBA2_VARIANT_RANGE": (("hba2", ("HBA2",)),),

    # --- marrow and malignancy 41-52 ---------------------------------------
    "MDS_SUSPECT": (("amc", ("WBC", "MONO")),),
    "ACUTE_LEUKAEMIA_SUSPECT": (("blasts_wbc", ("blasts_auer_rods", "WBC")),
                                ("blasts_plt", ("blasts_auer_rods", "PLT"))),
    # 43: path A needs SMEAR_SMUDGE_CELLS, absent from the 13-item registry.
    "CLL_CLPD_SUSPECT": (("alc_marked", ("WBC", "LYMPH")),),
    "CML_SUSPECT": (("wbc_abc", ("WBC", "BASO")),),
    "MPN_SUSPECT": (("thrombocytosis", ("PLT",)),
                    ("hgb_high", ("HGB",))),
    "MYELOMA_SUSPECT": (("rouleaux_bone_pain", ("HGB", "rouleaux")),
                        ("rouleaux_calcium", ("HGB", "rouleaux", "CALCIUM"))),
    "APLASTIC_ANAEMIA_SUSPECT": (
        ("pancytopenia_retic", ("HGB", "WBC", "PLT", "RETIC")),),
    "MARROW_PROCESS_SUSPECT": (("isolated_retic", ("HGB", "RETIC")),),
    "MARROW_INFILTRATION_SUSPECT": (
        ("leukoerythroblastic", ("teardrop_dacrocytes", "nucleated_rbc")),),
    "PROLYMPHOCYTIC_ATYPICAL_LYMPHOID_SUSPECT": (),
    "PANCYTOPENIA_SUSPECT": (("tri_lineage", ("HGB", "WBC", "PLT")),),
    "HLH_SUSPECT": (("bicytopenia", ("HGB", "PLT")),),

    # --- white-cell outcomes 53-60 -----------------------------------------
    "NEUTROPENIA_SUSPECT": (("anc_plt", ("WBC", "NEUT", "PLT")),),
    "AGRANULOCYTOSIS_SUSPECT": (("anc_severe", ("WBC", "NEUT")),
                                ("wbc_below_1", ("WBC",))),
    "BENIGN_ETHNIC_NEUTROPENIA_CONTEXT": (("anc_band", ("WBC", "NEUT")),),
    "LYMPHOPENIA_SUSPECT": (("alc_plt", ("WBC", "LYMPH", "PLT")),),
    "EOSINOPHILIA_SUSPECT": (("aec_mild", ("WBC", "EOS")),),
    "EOSINOPHILIA_INVESTIGATE": (("aec_mid", ("WBC", "EOS")),),
    "HYPEREOSINOPHILIA_SUSPECT": (("aec_marked", ("WBC", "EOS")),),
    "LEUKAEMOID_REACTION_SUSPECT": (("wbc_extreme", ("WBC",)),),

    # --- platelet and coagulation 61-66 ------------------------------------
    "THROMBOCYTOPENIA_SUSPECT": (("plt", ("PLT",)),),
    "INHERITED_THROMBOCYTOPENIA_SUSPECT": (("mpv", ("MPV",)),),
    "HIT_SUSPECT": (("plt_heparin", ("PLT",)),),
    "TMA_SUSPECT": (("schistocytes", ("PLT", "schistocytes")),),
    "DIC_SUSPECT": (("fibrinogen", ("PLT", "FIBRINOGEN")),
                    ("pt", ("PLT", "PT_PROLONGATION_SEC"))),
    "REACTIVE_THROMBOCYTOSIS_SUSPECT": (("plt_high", ("PLT",)),),

    # --- infection and regional 67-76 --------------------------------------
    "MALARIA_SUSPECT": (("anaemia_plt", ("HGB", "PLT")),),
    "DENGUE_SUSPECT": (("plt_wbc", ("PLT", "WBC")),),
    "KALA_AZAR_SUSPECT": (("anaemia", ("HGB",)),),
    "TB_ANAEMIA_SUSPECT": (("anaemia_amc", ("HGB", "WBC", "MONO")),),
    "HOOKWORM_STH_IDA_SUSPECT": (("micro_aec", ("MCV", "WBC", "EOS")),),
    "ENTERIC_FEVER_CONTEXT": (("leukopenia", ("WBC",)),),
    "SEPSIS_CONTEXT": (("plt_wbc", ("PLT", "WBC")),),
    "VIRAL_PATTERN_CONTEXT": (("leukopenia", ("WBC",)),),
    "HIV_CONTEXT_FLAG": (("anaemia", ("HGB",)),
                         ("leukopenia", ("WBC",)),
                         ("thrombocytopenia", ("PLT",))),
    "FUNGAL_MARROW_SUSPECT": (("pancytopenia", ("HGB", "WBC", "PLT")),),

    # --- systemic and other 77-80 ------------------------------------------
    "HYPERSPLENISM_CONTEXT": (("leukopenia", ("WBC",)),
                              ("thrombocytopenia", ("PLT",))),
    "HYPERTHYROID_ANAEMIA_SUSPECT": (("thyroid", ("HGB", "TSH", "FT4")),),
    "THYROID_RESULT_NEEDS_ACTION": (("tsh", ("TSH",)),),
    "CAPILLARY_SCREEN_CONFIRM_VENOUS": (("capillary_hgb", ("HGB",)),),

    # --- control and safety 81-88 ------------------------------------------
    "SEVERE_ANEMIA_SAFETY_BLOCK": (("hgb_severe", ("HGB",)),),
    # 82: four integrity routes.  Also selectable by missing/out-of-range
    # input, which is why it is additionally unconditional (see below).
    "CANNOT_EVALUATE": (("implied_mchc", ("MCH", "MCV")),
                        ("mchc_mismatch", ("MCHC", "MCH", "MCV")),
                        ("mchc_high", ("MCHC",)),
                        ("differential_sum",
                         ("NEUT", "LYMPH", "MONO", "EOS", "BASO"))),
    "NON_MICROCYTIC_ANEMIA_OUTSIDE_SCOPE": (("authored", ("MCV", "HGB")),),
    "ANEMIA_UNCLASSIFIED": (),
    "NO_FINDINGS": (
        ("all_normal", ("HGB", "MCV", "RBC", "MCH", "WBC", "PLT")),),
    "IDA_PROBABLE": (("rdwcv", ("HGB", "MCV", "RBC", "RDW_CV", "MCH")),
                     ("rdwsd", ("HGB", "MCV", "RBC", "RDW_SD", "MCH"))),
    "THALASSEMIA_TRAIT_PROBABLE": (("rdwcv", ("MCV", "RBC", "RDW_CV")),
                                   ("rdwsd", ("MCV", "RBC", "RDW_SD"))),
    "THALASSEMIA_TRAIT_POSSIBLE": (("rdwcv", ("MCV", "RBC", "RDW_CV")),
                                   ("rdwsd", ("MCV", "RBC", "RDW_SD"))),
}

# Outcomes an empty selection can still receive.  Both are engine behaviours,
# not clinical findings: an incomplete panel is exactly what makes them fire.
# They are declared here so the coverage denominator cannot be inflated by
# outcomes no selection had to earn.
UNCONDITIONAL_OUTCOMES = ("CANNOT_EVALUATE", "ANEMIA_UNCLASSIFIED")

# Arms present in the running engine that cannot be encoded from the declared
# 66 inputs.  Recorded rather than guessed at.
DROPPED_ARMS = (
    ("CLL_CLPD_SUSPECT", "alc_smudge",
     "reads SMEAR_SMUDGE_CELLS, which audit section 7.3 records as absent "
     "from the 13-item coded smear registry"),
    ("PROLYMPHOCYTIC_ATYPICAL_LYMPHOID_SUSPECT", "atypical_lymphoid",
     "reads SMEAR_ATYPICAL_LYMPHOID, absent from the coded smear registry; "
     "the outcome has no other arm and is therefore unreachable here"),
)

# Outcomes with no active emitting rule at all (audit section 1).
DEPRECATED_OUTCOMES = ("NORMAL_CBC", "THALASSAEMIA_TRAIT_SUSPECT")

# The audit's independent per-input verdict (section 7): does this input
# currently participate in selecting some outcome?  Used to check the encoding.
AUDIT_INERT_INPUTS = frozenset({
    "HCT",                                        # 7.1
    "ESR", "CREATININE", "eGFR", "POTASSIUM", "SODIUM", "UREA",
    "BILIRUBIN_TOTAL", "BILIRUBIN_INDIRECT", "AST", "ALT", "ALBUMIN",
    "D_DIMER", "APTT", "LDH", "MMA", "HOMOCYSTEINE", "TIBC", "STFR",
    "HBF", "G6PD_ASSAY",                          # 7.2, 20 external
    "pencil_cigar_cells", "howell_jolly_bodies",  # 7.3
    "BONE_PAIN",                                  # 7.4
})
AUDIT_N_DRIVING_INPUTS = 42
AUDIT_N_INERT_INPUTS = 24


def build_edges() -> list[Relationship]:
    """Expand ARMS into typed relationship edges.

    Every edge is `supporting` and carries its arm label in `path`, so members
    of an arm are conjunctive and arms are disjunctive.  Confidence is
    `derived`, not `defined`: the edges are mechanically derived from the
    audit's prose trigger descriptions rather than read from the rule YAML.
    """
    edges: list[Relationship] = []
    for _, outcome_id, _ in OUTCOMES:
        row = OUTCOME_ROW[outcome_id]
        for arm_label, inputs in ARMS[outcome_id]:
            for input_id in inputs:
                edges.append(Relationship(
                    input_id=input_id,
                    outcome_id=outcome_id,
                    rel_type="supporting",
                    path=f"{outcome_id}#{arm_label}",
                    source=f"{SOURCE_DOC}, row {row}",
                    confidence="derived",
                ))
    return edges


def build_profile() -> DomainProfile:
    """Construct the Q-BenchMed-Heme domain profile.

    Asserts the encoding against the audit's independent 42/24 input split
    before returning, so a transcription slip in any trigger row fails loudly
    here rather than silently changing a benchmark result.
    """
    missing_arms = set(OUTCOME_IDS) - set(ARMS)
    assert not missing_arms, f"outcomes without an ARMS entry: {sorted(missing_arms)}"
    assert len(INPUT_IDS) == 66, f"expected 66 inputs, got {len(INPUT_IDS)}"
    assert len(OUTCOME_IDS) == 88, f"expected 88 outcomes, got {len(OUTCOME_IDS)}"
    assert len(set(INPUT_IDS)) == 66, "duplicate input id"
    assert len(set(OUTCOME_IDS)) == 88, "duplicate outcome id"

    edges = build_edges()

    # Self-check against the audit's own per-input wiring verdict.
    driving = {e.input_id for e in edges}
    inert = set(INPUT_IDS) - driving
    unknown = driving - set(INPUT_IDS)
    assert not unknown, f"arms reference inputs outside the 66: {sorted(unknown)}"
    assert inert == AUDIT_INERT_INPUTS, (
        "encoding disagrees with audit section 7 per-input verdict; "
        f"unexpectedly inert: {sorted(inert - AUDIT_INERT_INPUTS)}; "
        f"unexpectedly driving: {sorted(AUDIT_INERT_INPUTS - inert)}"
    )
    assert len(driving) == AUDIT_N_DRIVING_INPUTS, (
        f"audit reports {AUDIT_N_DRIVING_INPUTS} outcome-driving inputs, "
        f"encoding gives {len(driving)}"
    )
    assert len(inert) == AUDIT_N_INERT_INPUTS

    profile = DomainProfile(
        benchmark_id=BENCHMARK_ID,
        domain="hematology",
        input_ids=list(INPUT_IDS),
        outcome_ids=list(OUTCOME_IDS),
        edges=edges,
        outcome_weights={o: 1.0 for o in OUTCOME_IDS},
        input_costs={i: 1.0 for i in INPUT_IDS},
        unconditional_outcomes=list(UNCONDITIONAL_OUTCOMES),
        objective_spec={
            "objective": "maximum_outcome_coverage",
            "outcome_weighting": "uniform",
            "input_cost_model": "uniform",
        },
        constraint_spec={"input_budget": None},
        provenance={
            "source_document": SOURCE_DOC,
            "knowledge_version": KNOWLEDGE_VERSION,
            "real_export_present": True,
            "synthetic_stand_in": False,
            "confidence_default": "derived",
            "cost_model_justified": False,
            "ambient_context_fields": list(AMBIENT_CONTEXT),
            "derived_value_expansions": {k: list(v) for k, v in DERIVED.items()},
            "dropped_arms": [
                {"outcome": o, "arm": a, "reason": r} for o, a, r in DROPPED_ARMS
            ],
            "deprecated_outcomes": list(DEPRECATED_OUTCOMES),
            "outcome_wiring": dict(OUTCOME_WIRING),
            "acquisition_tier": dict(ACQUISITION_TIER),
            "audit_input_split": {
                "outcome_driving": AUDIT_N_DRIVING_INPUTS,
                "inert": AUDIT_N_INERT_INPUTS,
            },
        },
        notes=(
            "Derived from an implementation audit of a running rule engine, not "
            "from a clinical specification. Edges describe which inputs the "
            "engine currently reads. No clinical claim follows from any "
            "optimization result on this profile: a smaller covering panel does "
            "not mean a test is clinically unnecessary."
        ),
    )
    return profile
