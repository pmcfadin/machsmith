# Round 2 design: refereed speed-ups on CQLite's hot spots

Status: draft, 12 November 2026. Source brief: `docs/prompts/2026-11-12-round-2-cqlite-hotspots.md`.
The owner asked the controller to pick sensible defaults for the open decisions instead of settling
them one by one. Every such choice is marked **(controller call)** and is listed in the decision
ledger (section 11) with its cost if wrong. Nothing here is owner-agreed except where the brief says so.

## 1. Why

Machsmith's long-term goal is a PMD-style tool that a coding agent runs, acts on and reruns to make
Rust faster. Round 1 showed codegen-level wins are small and situational, that hand-picked kernels were
not hot spots, and that the referee (null band, noise gate, session bootstrap) is the part that earns its
keep. Round 2 asks two things:

1. Can an agent, guided by a profile and checked by a strict referee, make CQLite's own benchmarked
   operations measurably faster with source-level Rust changes?
2. Can we write down, **before** each fix, the rule a linter would have used to find it, and does the
   rule's predicted gain hold?

## 2. Goals, non-goals, anti-goals

Goals:
- A fresh, stable profile of current CQLite on the chosen workloads, ranked by share.
- For each hot spot attempted: a pre-registered rule, one branch with one fix, a refereed verdict and
  the rule's prediction checked against it.
- A results write-up in `docs/results/` and a rule catalogue in `docs/rules/`.

Non-goals:
- A reusable toolkit. Round 2 writes rules down; it does not build the linter.
- Assembly, PGO, target-cpu or other compiler levers (round 1 covered them).
- Speeding up anything CQLite's benchmarks do not measure.

Anti-goals (a result is a failure even if it "works"):
- A claimed win smaller than T, or a win with no recorded run behind it.
- A rule written after the fix, dressed up as a prediction.
- A measurement rule changed after results were seen without stopping to ask the owner (round 1's
  process error).
- Any change to `/home/dev/projects/cqlite`, to round 1's artifacts, or to the shared cargo target dir.
- "CQLite is X% faster" stated beyond the operation measured.

## 3. Fixed constraints (from the brief)

- CQLite is read only. Work in a clone at `work/cqlite-r2`, at CQLite HEAD
  `7f9abe3663254724439dbe71aff77bb93faa1c2d` (checked 12 November 2026; 3 commits after round 1's
  `2e5b7a4c`). Record the commit in every measurement record. One branch per hot spot, named
  `r2/<rule-id>`, each cut from that commit.
- `CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets` for every bench and test run.
- Builds only via `experiment/env.sh` and `msx_cargo <name>`, into `work/target/<name>`. Toolchain 1.97.1.
- Round 1 artifacts untouched: `experiment/results/`, `work/target/tool/release/msx`,
  `docs/results/2026-11-llm-asm-experiment.md`, `work/cqlite`, and (controller call) `tools/` as well,
  since round 1's write-up cites them. Round 2 code lives in `round2/`.
- Changes stay internal to CQLite: no public API, on-disk format or wire change (section 8).
- No system installs, pushes, merges or PRs. Machsmith commits go to local `main`; ask before pushing.
- No own builds during own measurement windows; keep load light while anything else measures.

## 4. Workloads **(controller call)**

Three workloads, each one of CQLite's Criterion bench targets, run with all their cases:

| workload | bench target | why |
|---|---|---|
| `read` | `read` (9 cases) | user-visible SSTable reads; 6 of its cases are STRICT in `perf-gate.json` |
| `decode` | `decode` (31 cases) | value decoding; 3 cases STRICT in `perf-gate.json` |
| `partition_lookup` | `partition_lookup` | index lookup; not in `perf-gate.json`, kept because round 1 profiled it |

No fourth workload is added. `export_throughput` and `open` are user-visible too, but adding them
lengthens every verdict run and the brief's hypotheses are about the three above. A hot spot found in
these three that also plausibly affects `export_throughput` is noted in the write-up, not measured. If
the owner wants `export_throughput` in, it joins as a fourth "other workload" report only.

## 5. Profiling

- Profile **the same binary the referee measures**: CQLite's `bench` profile as `cargo bench` builds it
  (`[profile.bench] debug = true`, symbols kept). CQLite's own `Cargo.toml` notes that debuginfo moves
  `.text`, so profiling `perfsym` while measuring `bench` would take target cases and predictions from one
  layout and verdicts from another **(controller call)**. Round 1 profiled `perfprof`; round 2's shares
  are therefore not comparable 1:1 with round 1's. A `--call-graph dwarf` run on the same binary is
  allowed and labelled structural evidence.
- `perf record -e cycles:u -F 4999`, pinned to one core, `--profile-time 20` per case (10x round 1's
  2 s **(controller call)**). Report duration and sample counts per workload.
- Stability check: profile each workload twice, at different times. A hot spot is "confirmed" when its
  share differs by less than 2 percentage points between the two profiles and is at least 3% in both.
- Attribution: copy `tools/ceiling.py` into `round2/` and extend it to rank **call-tree** shares (callers
  of allocator, hashing, `bcmp`, type parsing) as well as round 1's innermost-function buckets, since
  round 1's buckets put inlined std code under `other`. Copy `tools/profile-cqlite.sh` to
  `round2/profile.sh` parameterized by clone and profile, and fix its misplaced `perf script
  --show-mmap-events` line (it sits inside the "no binary" error branch).
- Quiet-host gate applies to profiling runs too (section 6.4).

Output: `round2/results/profile/{read,decode,partition_lookup}.{json,md}` and a ranked hot-spot table.

## 6. The referee

### 6.1 What is measured

The verdict is on CQLite's own Criterion benchmark, end to end, for the operation the hot spot sits in.
Both arms are built the same way: `cargo bench --no-run` with CQLite's default `bench` profile and the
same features (`cli-helpers,bench-internals` where the target needs them), from the base commit
(baseline) and from the branch (candidate), each into its own `work/target/r2-<arm>-<sha>`.

- **Primary metric:** Criterion's per-iteration wall time, median over its samples, per case.
- **Secondary, always recorded:** `perf stat -e cycles:u,instructions:u` around each bench process, and
  per-case Criterion mean and MAD.

### 6.2 Sessions

A **session** runs, on one pinned core and with a fresh process each, the baseline binary and the
candidate binary over the selected cases, in a seeded random order (seed from round 1's scheme:
`0x6D7378` base XOR a per-run nonce). Each process gets its own `CRITERION_HOME` and runs with fixed
`--warm-up-time 1 --measurement-time 3 --sample-size 50 --noplot`. ASLR stays on; environment size is
randomized per process, as in round 1.

A session measures **only the hot spot's own workload** (all its cases, so target and whole-workload
statistics come from the same sessions). The other two workloads are measured in a separate
**cross-check run** of 10 sessions under the same protocol, after the confirmation run; it can flag
REGRESSION but never issues the branch's verdict **(controller call)**. This keeps one confirmation run
to roughly 1 to 3 hours instead of an estimated 3 hours for all 46 cases.

Per session and case: `ratio = candidate median / baseline median`. Per session and workload: the
geometric mean over the workload's **target cases** (6.5). The verdict interval is a 95% percentile
bootstrap of the geometric mean over sessions (10,000 resamples, seed `0x6D7378`), reusing
`experiment/harness/src/stats.rs` as a copied module in `round2/referee/`.

Counts: **confirmation runs** use 20 valid sessions. **Screening runs** during development use 5
sessions and never produce a reported verdict.

### 6.3 Null run and threshold

Before the first verdict, and again before any verdict run more than 7 days after the last null:
an A/A run with the baseline binary against a byte-identical copy at a different path, full protocol,
all three workloads (20 sessions each). The null band is computed **on the same statistic the verdict
uses**: for each branch, `b` = the largest distance from 1.0 of either bound of (a) the null's
geometric mean over that branch's pre-registered target cases and (b) the null's whole-workload
geometric mean for that workload. No extra run is needed; the null's per-session per-case ratios are
recomputed for each target set. This is fixed now, before any null run **(controller call)**.
`T = max(5%, 2 × b)` per branch, reused from round 1 rather than `perf-gate.json`'s 10%
**(controller call)**: round 1's T is the stricter, interval-based rule, and 10% is a CI regression
wall, not a claim threshold. `perf-gate.json`'s 10% per-case regression rule is still reported as an
extra column for the owner's CI view.

If `b > 5%` on three null tries at different times, the ruler is broken: stop and report.

Ruler calibration, run once after the first null (council finding, round 1 spec section 12):
- a **planted slowdown** branch (a `black_box` spin on a hot `read` path, sized to cost about 15%) must
  come out LOST;
- a **planted no-op** branch (a behaviour-free source change that changes the binary, e.g. a renamed
  private function) must come out TIED.
If either fails, stop and report.

### 6.4 Noise gate

- **Quiet-host precondition** before every profile, null, screening and confirmation run: no `rustc` or
  `cargo` process on the host and 1-minute load below 2.0, both held for 10 consecutive minutes.
  Record load averages, the process snapshot and the chosen core at start.
- **Per-session hygiene:** a session is rejected if, during it, the 1-minute load rose above 3.0, any
  `rustc`/`cargo` process appeared, the measuring core's SMT sibling exceeded 20% utilization (round
  1's `MAX_SIBLING_UTIL`), or either bench process logged more than 50 involuntary context switches
  per second of run time.
  **Amended 2026-11-13 (owner-approved, DECISIONS R15):** the context-switch criterion is replaced by:
  for each arm (one `taskset … perf stat … <bin>` process), `core_busy_s` = busy time of the pinned core
  between spawn and reaping (`/proc/stat` jiffies delta over `sysconf(_SC_CLK_TCK)`), `own_cpu_s` =
  `ru_utime + ru_stime` from `wait4`, `foreign_s = max(0, core_busy_s - own_cpu_s)`, `foreign_frac =
  foreign_s / wall_s`; the session is rejected when either arm's `foreign_frac` exceeds 0.02
  (`MAX_FOREIGN_CPU_FRAC`). Precedence: load, cargo/rustc, sibling, foreign CPU. When the core's
  time is unavailable or wall time is 0 the rule does not apply. Reason: the benches' own pinned
  multi-threaded runtime causes the switches (R2-000005..7).
- **Run-level gate:** at most 3 rejected sessions are replaced per run (round 1's
  `MAX_SESSION_RERUNS`); a 4th rejection ends the run as `NOISE_GATE_FAILED` with no verdict.
- **Retry cap, fixed now:** at most 3 confirmation tries per branch, each after a fresh quiet-host
  window. After 3 failed tries the branch is reported "unmeasurable on this host on this date". This
  cap is not raised during the round without stopping to ask the owner.

### 6.5 Target cases (pre-registered)

Before the change is written, the rule entry (section 7) names the **target cases**: the cases of the
hot spot's workload where the profile shows the hot spot's share is at least 3%. The verdict is on the
geometric mean over target cases. Also reported, without changing the verdict:
- the whole-workload geometric mean and verdict;
- per-case ratios and intervals for every case of the workload, and of the other two workloads from the
  cross-check run (6.2);
- any case whose interval is entirely above `1 + T` is flagged **REGRESSION**.

A branch that WINS on its target but has a REGRESSION elsewhere is reported as "won with regression" and
is not recommended to the owner as-is.

### 6.6 Verdicts

Round 1's table: ratio = candidate / baseline; WON if `hi < 1 − T`, LOST if `lo > 1 + T`, TIED if
`lo ≥ 1 − T` and `hi ≤ 1 + T`, INCONCLUSIVE otherwise. Each verdict reports `b`, `T`, ratio and 95%
interval.

### 6.7 Records

Append-only JSONL ledger at `round2/results/ledger.jsonl`, one record per run (profile, null,
calibration, screening, confirmation, test-suite run), with: ledger id `R2-NNNNNN`, kind, CQLite commit
and branch sha, binary sha256s, host state at start, session details and rejections, per-case results,
verdicts, and machsmith commit plus a dirty flag that lists dirty paths (round 1's boolean flag could
not say what was dirty). Never edit a record; corrections are new records that reference the old one.
Every number in the write-up cites a ledger id.

## 7. Hot spots and rules

### 7.1 How many **(controller call)**

Attempt hot spots in rank order from the fresh profile, at most **4 fixes**, and stop earlier when the next
candidate's predicted gain is below T. Each gets at most **2 implementation attempts** (screened), and
then exactly one confirmation run (plus noise-gate retries). This separates search from confirmation,
as the round 1 council asked. A hot spot whose only fix is a public-API change (section 8) becomes a
proposal: it does **not** count toward the 4, the agent moves on to the next-ranked spot, and the
proposal gets a sizing run only after every fix branch's confirmation and cross-check runs are done
**(controller call)**.

### 7.2 Expected candidates (hypotheses from round 1, to be confirmed)

| rank guess | workload | hypothesis | likely internal fix | API risk |
|---|---|---|---|---|
| 1 | `decode` | `CqlType::parse_with_depth` re-parses type strings per value (~55%) | parse once, cache the parsed type | check `decode_value_for_bench` and the comparator path signatures |
| 2 | `read` | allocator ~31%, memcpy 7%, `Arc<str>` clone/drop ~10% | avoid per-row/per-cell allocation and clones | `Value` is public; owned/borrowed shape must not change |
| 3 | `partition_lookup` | SipHash ~22% | faster hasher (`rustc-hash` is already a dependency) | **`IndexReader::key_lookup` is a `pub` field of type `HashMap<Arc<[u8]>, usize>` in a `pub mod`**, so changing its hasher changes the public API. Likely a proposal, not a fix, unless an internal-only route exists |
| 4 | `partition_lookup` | `bcmp` ~20% | key comparison cost; may fall out of the hasher change | as above |

### 7.3 Rule entry, written before code

`docs/rules/<rule-id>.md`, committed before the branch's first code commit:
- id and name;
- the pattern, and how to detect it from source, MIR or machine code;
- evidence that it is hot (profile record ids and shares);
- target cases (6.5);
- predicted gain with its reasoning (for example, an Amdahl bound from the share and the expected
  fraction removed), stated as a ratio range;
- later, appended (not edited): the verdict, whether the prediction held (actual ratio inside the
  predicted range), and whether it paid off.

"Prediction held" is defined now: the confirmation point estimate falls inside the predicted range.

### 7.4 Correctness

"Tested by CQLite's suite only." On each branch, before its confirmation run: CQLite's agent gate in
full mode (`scripts/agent-gate.sh`, run detached per CQLite's `CLAUDE.md`), whose summary block is the
verdict. If the gate cannot certify in a clone (for example its rebase or tree-integrity checks
refuse), fall back to `cargo test --workspace` plus
`RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --all-features`, record that as a
deviation, and say so in the write-up **(controller call)**. The base commit's suite is run the same
way first, so pre-existing failures are known. A branch with any failure not present on the base does
not get a confirmation run. Test builds go to `work/target/r2-test`, never during a measurement window,
and run under `nice -n 10`.

## 8. Public API **(controller call)**

No public-API change is made this round. CQLite's public surface means anything reachable as `pub`
from the crate root of any published crate, including `pub` fields. When the best fix needs such a
change, the agent writes `docs/rules/proposals/<rule-id>.md` with the evidence and the predicted gain,
and may measure it on a branch marked `r2-proposal/<rule-id>` **only** to size the proposal; such a
measurement is labeled "proposal, not a fix" and is never counted as a round 2 win. Making the change
for real needs the owner's approval.

## 9. Deliverables

1. `round2/`: profile scripts, attribution, referee (copied stats, new whole-bench session runner),
   ledger, results.
2. `docs/rules/`: one entry per rule, plus `proposals/`.
3. `docs/results/2026-11-round-2-cqlite-hotspots.md` in round 1's style: every number cites a ledger id;
   correctness "tested by CQLite's suite only"; no claim smaller than T; what did not win and why;
   limitations (one shared host, small cache-resident data, wall time not cycles as the primary metric,
   bench-profile build).
4. `round2/DECISIONS.md`: the decision ledger (section 11), kept current through execution.
5. A short message to the owner: what got faster, by how much with ledger ids, which rules held, what
   is left.

## 10. Stop-and-ask points

- anything irreversible; pushing or publishing; any public-API change;
- changing any measurement rule (sessions, T, gate thresholds, retry cap, target cases) after seeing a
  result, including after a null or calibration run;
- ruler broken (6.3) or calibration failed;
- base commit's test suite failing in a way that blocks every branch;
- CQLite's HEAD moving is **not** a stop: round 2 stays on `7f9abe36`.

## 11. Decision ledger (initial)

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

## 12. Left to the builder

Exact file layout inside `round2/`; whether the referee is Rust (reusing `stats.rs` directly) or Python
(porting it, with a test that it matches round 1's outputs on round 1's ledger); how the planted
slowdown is sized; the call-tree attribution method; and report formatting.
