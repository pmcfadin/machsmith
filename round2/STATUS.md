# Round 2 status

One line per close-out task (`docs/plans/2026-11-12-round-2/phase-2b-closeout.md`). Updated in every
task's commit. Ledger: `round2/results/ledger.jsonl`.

| task | state | commit | ledger ids | notes |
|---|---|---|---|---|
| 1 Close Task 9 (profiles A, B, A-cg; hot-spot ranking) | done | `bf5bdcb` | R2-000011, R2-000012, R2-000013 | `results/profile/HOTSPOTS.md` |
| 2 Pre-register owner decisions | done | `2ee13b7` | — | DECISIONS R20–R24; `results/attempts-null.md` |
| 3 Branch-sampling feasibility gate | done | `81c7a0c` | — | R25 (feasible), R26 (perf 7.0.14 pinned) |
| 4 Branch-stack profiling and caller attribution | done | `28fd445` | — | `callers.py`; R27 |
| 5 Decode profile C and read branch stacks | done | `284487c` | R2-000014 (C), R2-000015 (A-br) | |
| 6 H2 verdict, H3/H5 call sites | done | `a8094ab` | — (reads R2-000014, R2-000015) | H2 not confirmed (C 69.63 vs 64.31 / 61.12), skipped per R22; H3 no rule; H5 rule candidate `bti_point.rs:787` |
| 7 Null runs (read, decode, partition_lookup) | in progress | — | R2-000016 (read: OK, b 0.0145, T 0.05); R2-000017 (decode: NOISE_GATE_FAILED, R21 1 of 3) | attempts in `results/attempts-null.md`; decode retry and partition_lookup pending |
| 8 Ruler calibration | not started (needs Task 7) | — | — | |

Usability fixes (2026-11-21): `preflight.sh` (R28), early env check in the Python tools (R29), this file.
