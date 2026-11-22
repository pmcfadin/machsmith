# Ruler check: null bands and calibration

## Sizing screens for `r2/cal-slow` (provisional)

5-session screening runs used only to size `CAL_SPIN` so that the planted slowdown's whole-workload
ratio lands in 1.12–1.18. **Provisional: these are not results.** Base `work/r2-bins/7f9abe366325/read`
(`773f4a2e…`), null `R2-000016`.

| # | `CAL_SPIN` | `r2/cal-slow` commit | ledger id | sessions (rejected) | whole-workload ratio (interval) | in 1.12–1.18? |
|---|---|---|---|---|---|---|
| 1 | 64 | `edc3ba2d` | R2-000023 | 5 (0) | 1.0057 (1.0024–1.0095) | no: too small |
| 2 | 1300 | `8cfe0190` | R2-000024 | 5 (0) | 1.1015 (1.0543–1.1349) | no: too small |
