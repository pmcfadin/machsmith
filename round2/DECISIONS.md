# Decision ledger

Append rows; never edit a past row's decision — add a superseding row.

| # | decision | why | cost if wrong |
|---|---|---|---|
| D1 | Three workloads, no fourth | brief's hypotheses; keeps verdict runs bounded | miss a user-visible win in export/open |
| D2 | At most 4 hot spots, stop when predicted gain < T | bounded effort; low-share spots cannot clear T | stop before a surprise win |
| D3 | Round 1's T, not `perf-gate.json` 10% | stricter and interval-based; 10% is a CI wall | none for claims; owner may want the 10% view, which is reported |
| D4 | No public-API change; proposals only | brief's default; owner confirms API changes | the SipHash fix may be blocked by the `pub` field |
| D5 | Profile the `bench`-profile binary the referee measures | profile and verdict see the same code layout | `bench` has debuginfo, so its layout differs a little from a plain release build users run |
| D6 | Primary metric is Criterion wall time per iteration | it is CQLite's own benchmark; cycles only around whole processes | wall time carries frequency noise; the null band absorbs it |
| D7 | Verdict on pre-registered target cases | a fix that touches 2 of 31 cases cannot move a 31-case geomean past T | target selection could be gamed; it is fixed before code |
| D8 | 20 confirmation sessions, 5 screening sessions | round 1 used 30 on cheap kernels; whole benches are slower | wider intervals; more TIED |
| D9 | 3 confirmation tries per branch, fixed now | round 1's error was changing the cap mid-run | a branch may end unmeasurable |
| D10 | Agent gate as the test suite, with a recorded fallback | CQLite's `CLAUDE.md` names it the gate of record | gate may not run in a clone; fallback is weaker |
| D11 | Leave `tools/` untouched, copy into `round2/` | round 1's write-up cites them | small duplication |
| D12 | Null band per branch, on the verdict's own statistic | a band from whole-workload means would be too narrow for a 1 to 3 case target mean | T may be larger than 5% for small target sets, giving more TIED |
| D13 | Confirmation measures only the hot spot's workload; others in a 10-session cross-check | halves the quiet window needed | a regression elsewhere is seen with less precision |
| D14 | Proposals don't count toward the 4 fixes; sized last | quiet-host time is the scarce resource | a proposal may go unmeasured if time runs out |
| D15 | No coordination with other agents; rely on the quiet gate, schedule runs at low-activity hours | the controller cannot pause other agents | branches may end "unmeasurable"; the owner can arrange quiet windows to prevent it |
| R1 | `r2` accepts `R2_TEST_MODE=1`, which skips the quiet-host wait and uses an always-quiet probe; every record written in test mode carries `"test_mode": true`, and `r2` refuses test mode when the ledger path resolves to `round2/results/ledger.jsonl` | tests must drive the CLI; the guard keeps test mode out of real records | a test-only path in the referee binary |
| R2 | `round2/build.sh` uses one shared target dir `work/target/r2-bench` for all CQLite revisions, copying each binary out to `$R2_BINS/<sha12>/` at once | avoids a multi-GB full rebuild per revision on a shared host | an incremental build could differ from a clean one; the same-revision-twice reproducibility check still guards it |
| R3 | Task 2's test keeps a byte-identical copy of round 1's `bootstrap_ci` as a frozen oracle | proves the port is exact | about 30 duplicated test lines |
| T5.1 | The involuntary-context-switch gate judges `Probe::ctx_switches(measured)`; `ProcProbe` passes `wait4`'s `ru_nivcsw` through, the always-quiet test-mode probe reports 0; records keep both `nivcsw` (measured) and `nivcsw_gated` | on this shared host `perf stat` start-up alone costs 10-40 involuntary switches, so sub-second fake benches tripped the 50/s gate and made tests flaky; R1 already fakes every other host-noise input in test mode | the gate's wait4 plumbing is covered by a test probe that inflates the count, not by real host noise |
| T5.2 | A null run is refused unless both arms are byte-identical files at different paths, and takes no targets | spec 6.3 defines the null as an A/A run against a byte-identical copy; targets are applied later by `nullband` | none known |
| T5.3 | `r2 run` takes `$R2_LOCK` with a 5 s grace and then fails without a record; it does not queue | a run that waits for hours behind another run would start in an unknown host state | a second run must be restarted by hand |
| T5.4 | Session rejection precedence: process failure, then missing case, then host monitor; a failed first arm skips the second | deterministic binary defects are reported before noise | none for verdicts (any rejection replaces the session) |
| T5.5 | Calibration and proposal runs default to 20 sessions; `--sessions` may differ from the protocol count but the record then carries `"protocol_deviation": true` | tests and the ruler check need short runs; the flag keeps the write-up honest | a deviated run could be cited by mistake; the flag is in every record |
| T5.6 | A quiet-host wait that times out ends the run as `NOISE_GATE_FAILED` with a record | the quiet precondition is part of the noise gate (spec 6.4) | none |
