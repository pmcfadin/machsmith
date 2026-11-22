# Null attempts

One row per null attempt, including attempts killed with no ledger record. Counting rules:
DECISIONS R20 (3-try cap: completed nulls with `b > 0.05` only) and R21 (noise-gate soft cap: at
most 3 `NOISE_GATE_FAILED`, `aborted` or killed attempts per workload after R21's commit, ≥ 2 h
apart). "started" is the host snapshot time at the run's start.

| workload | attempt | started (UTC) | ledger id | status | b | counts toward |
|---|---|---|---|---|---|---|
| read | pre-1 | 2026-11-13T11:00:49Z | R2-000005 | NOISE_GATE_FAILED | — | pre-R20, counts toward nothing |
| decode | pre-1 | 2026-11-13T11:18:24Z | R2-000006 | NOISE_GATE_FAILED | — | pre-R20, counts toward nothing |
| partition_lookup | pre-1 | 2026-11-13T11:48:23Z | R2-000007 | NOISE_GATE_FAILED | — | pre-R20, counts toward nothing |
| read | pre-2 | 2026-11-13T21:36:42Z | R2-000008 | NOISE_GATE_FAILED | — | pre-R20, counts toward nothing |
| decode | pre-2 | 2026-11-13T22:09:51Z | R2-000009 | NOISE_GATE_FAILED | — | pre-R20, counts toward nothing |
| partition_lookup | pre-2 | 2026-11-13T22:57:54Z | R2-000010 | NOISE_GATE_FAILED | — | pre-R20, counts toward nothing |
| read | pre-3 | 2026-11-14T01:08Z | — | killed, no record | — | pre-R20, counts toward nothing |
| read | 1 | 2026-11-21T18:57:49Z | R2-000016 | OK (20 sessions, 1 rejected: load 4.19) | 0.0145 (T 0.05) | done: b ≤ 0.05; 0 counted tries (R20), 0 toward R21 |
| decode | 1 | 2026-11-21T19:41:42Z | R2-000017 | NOISE_GATE_FAILED (17 sessions, 4 rejected: load 4.07, 4.76, 4.53, 6.01) | — | R21: 1 of 3; 0 counted tries (R20). Next decode attempt not before 21:41:42Z |
| partition_lookup | 1 | 2026-11-21T21:36:54Z | R2-000018 | OK (20 sessions, 0 rejected) | 0.0813 (T 0.1626) | **counted try 1 of 3** (R20: b > 0.05); 0 toward R21. Next attempt not before 23:36:54Z |
| decode | 2 | 2026-11-21T22:18:57Z | R2-000019 | NOISE_GATE_FAILED (12 sessions, 4 rejected: load 4.17, 4.22, 4.06; cargo/rustc seen) | — | R21: 2 of 3; 0 counted tries (R20). Next decode attempt not before 2026-11-22T00:18:57Z; a third failure is stop-and-ask |
| partition_lookup | 2 | 2026-11-22T09:41:19Z | R2-000020 | NOISE_GATE_FAILED (2 sessions, 4 rejected: load 4.51, 4.39, 10.4, 8.82; 10 h in the quiet wait first) | — | R21: 1 of 3; counted tries stay at 1 of 3 (R20). Next attempt not before 11:41:19Z |
| decode | 3 | 2026-11-22T10:48:39Z | R2-000021 | OK (20 sessions, 3 rejected: load 4.03, 4.12, 4.01) | 0.0219 (T 0.05) | done: b ≤ 0.05; 0 counted tries (R20); R21 stays at 2 of 3 |
