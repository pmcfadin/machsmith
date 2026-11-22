# Ruler check: null bands and calibration

Close-out Tasks 7 and 8 (2026-11-21/22). Every figure is from `round2/results/ledger.jsonl`; referee
`work/target/r2-tool/debug/r2` was not rebuilt between these runs. Base binaries are CQLite `7f9abe36`
(`results/profile/binaries.json`). Kernel 7.0.0-43, perf 7.0.14 (R26). Per-session load limit 4.0 (R18).

**Outcome: the ruler passed.** All three nulls are `OK` with b ≤ 0.05; the planted slowdown is `LOST`
and the planted no-op is `TIED`.

## Null bands

`b` is the whole-workload null band (`max(1 − lo, hi − 1)` of the null's interval), `T = max(0.05, 2b)`.
Candidate = a byte-identical copy of the base binary (sha256 checked equal before the first run).

| workload | null | sessions (rejected) | null interval (lo / point / hi) | b | T | core | started (UTC) | load 1/5/15 at start |
|---|---|---|---|---|---|---|---|---|
| read | R2-000016 | 20 (1) | 0.9855 / 0.9917 / 0.9975 | 0.0145 | 0.05 | 4 | 2026-11-21 18:57 | 1.23 / 1.71 / 5.77 |
| decode | R2-000021 | 20 (3) | 0.9972 / 1.0082 / 1.0219 | 0.0219 | 0.05 | 8 | 2026-11-22 10:48 | 0.27 / 1.24 / 4.51 |
| partition_lookup | R2-000022 | 20 (0) | 0.9584 / 0.9860 / 1.0046 | 0.0416 | 0.0832 | 8 | 2026-11-22 12:48 | 0.35 / 0.52 / 0.93 |

Tries (full list in `attempts-null.md`):

| workload | attempts | counted tries (R20, cap 3) | noise-gate failures (R21, cap 3) |
|---|---|---|---|
| read | 1 | 0 | 0 |
| decode | 3 | 0 | 2: R2-000017 (4 rejected: load 4.07–6.01), R2-000019 (4 rejected: load 4.06–4.22, cargo/rustc seen) |
| partition_lookup | 3 | 1: R2-000018 (OK, b 0.0813, 0 rejected) | 1: R2-000020 (4 rejected: load 4.39–10.4) |

Things a reader should know before using these bands:

- **read's null interval does not contain 1** (0.9855–0.9975): the copy measured 0.8% faster than the
  binary it was copied from. The band covers it (b 0.0145 < T 0.05), but it is a position or order effect
  of that size, not zero. Per case, `read/clustering_slice` is 0.982–0.989.
- **decode:** six cases have null intervals that exclude 1 (`type_date`, `type_decimal`, `type_inet`,
  `type_timeuuid`, `type_uuid` above 1 by up to 3%; `type_frozen` 0.945–0.996). Any per-case or
  target-set claim on decode needs its own band from `r2 nullband --targets`.
- **partition_lookup is the widest ruler**, and its T is 0.0832, not the 0.05 floor: no claim smaller
  than 8.3% can be made on it. Its first completed null (R2-000018) had b 0.0813 with no session
  rejected and start load 1.67/1.37/1.87, so that width was not flagged by the noise gate. The two
  completed nulls differ by a factor of two in b.
- The host was busy for most of the campaign: R2-000020 waited about 10 h for the quiet precondition.

## Calibration (read, against null R2-000016, T 0.05)

| run | candidate | ledger id | sessions (rejected) | ratio (lo / point / hi) | verdict | expected | core | started (UTC) | load 1/5/15 at start |
|---|---|---|---|---|---|---|---|---|---|
| planted slowdown | `r2/cal-slow` `3a538d87`, `CAL_SPIN = 1700`, binary `8c66dd1f…` | R2-000027 | 20 (0) | 1.1535 / 1.1665 / 1.1801 | **LOST** | LOST | 12 | 2026-11-22 15:15 | 1.33 / 0.90 / 0.97 |
| planted no-op | `r2/cal-noop` `db9f82a8`, binary `dfc4b630…` | R2-000028 | 20 (3: load 4.27, 4.27, 4.19) | 0.9982 / 1.0031 / 1.0080 | **TIED** | TIED | 2 | 2026-11-22 15:56 | 0.29 / 0.57 / 1.04 |

- `r2/cal-slow`: a `std::hint::black_box` spin of `CAL_SPIN` iterations at the top of
  `V5CompressedLegacyParser::parse_row_data_with_offset_impl` (`row_decoder/row_data.rs`). The slowdown
  is uneven across cases by design (in sizing screen 3, 1.05 for `get_partition_big` to 1.74 for
  `clustering_slice`); the 1.12–1.18 target is on the whole-workload ratio.
- `r2/cal-noop`: private `parse_clustering_value` renamed to `decode_clustering_value`
  (`row_decoder/row_framing.rs`, one definition, one call site). Its binary's sha256 `dfc4b630…` differs
  from the base's `773f4a2e…`.
- Both branches were checked by a fresh reviewer before any build. Neither calibration hit the noise gate.
- What this does and does not show: the referee calls a 17% whole-workload slowdown `LOST` and a no-op
  `TIED` on read. It was not calibrated on decode or partition_lookup, and no effect near T was planted.

## Sizing screens for `r2/cal-slow` (provisional)

5-session screening runs used only to size `CAL_SPIN` so that the planted slowdown's whole-workload
ratio lands in 1.12–1.18. **Provisional: these are not results.** Base `work/r2-bins/7f9abe366325/read`
(`773f4a2e…`), null `R2-000016`.

| # | `CAL_SPIN` | `r2/cal-slow` commit | ledger id | sessions (rejected) | whole-workload ratio (interval) | in 1.12–1.18? |
|---|---|---|---|---|---|---|
| 1 | 64 | `edc3ba2d` | R2-000023 | 5 (0) | 1.0057 (1.0024–1.0095) | no: too small |
| 2 | 1300 | `8cfe0190` | R2-000024 | 5 (0) | 1.1015 (1.0543–1.1349) | no: too small |
| 3 | 2000 | `ebf2ffdc` | R2-000025 | 5 (0) | 1.1875 (1.1815–1.1947) | no: too large |
| 4 | 1700 | `3a538d87` | R2-000026 | 5 (0) | 1.1706 (1.1616–1.1775) | **yes**: sized |
