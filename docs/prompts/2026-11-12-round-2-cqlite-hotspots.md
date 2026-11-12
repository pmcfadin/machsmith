# Round 2 agent prompt: real speed-ups on CQLite's hot spots, refereed

Paste everything below the line into a fresh agent session started in `/home/dev/projects/machsmith`.

---

You are starting round 2 of machsmith. Machsmith's long-term goal is a PMD-style tool that a coding agent runs, acts on, and runs again to make Rust code faster. Round 1 tested whether an LLM can beat the compiler on isolated kernels. Round 2 tests whether an agent, guided by a profile and checked by a strict referee, can make **CQLite itself** measurably faster. It also tests whether we can write down, before each fix, the rule a linter would have used to find it.

## Read first, in this order

1. `docs/results/2026-11-llm-asm-experiment.md`: round 1's write-up. Read sections 1, 3 and 8 closely.
2. `docs/specs/2026-11-12-llm-asm-experiment-design.md`, sections 1-2, 10 and 12: the question, the limitations, and the review panel's findings carried forward to the toolkit.
3. `experiment/results/profile/{read,decode,partition_lookup}.json` and `ceiling.md`: round 1's profile of CQLite. It is indicative only: 2 s per bench case, at CQLite commit 2e5b7a4c, and CQLite has moved since.
4. `tools/profile-cqlite.sh`, `tools/ceiling.py` and `experiment/env.sh`: the profiling path that already works on this host.
5. In the CQLite repo (read only), `cqlite-core/benches/README.md`, `cqlite-core/benches/perf-gate.json` and `cqlite-core/benches/bench_ledger/`. CQLite already has benchmark and performance-gate infrastructure. Learn it, and reuse it where it fits rather than inventing a parallel system.

## What round 1 established (do not re-derive)

- **Codegen-level wins are real but small and situational.** The LLM-written asm beat rustc's best output (opt 3, znver5, PGO) by about 13% on `vint` only when the function is called out of line. The compiler's inlined version was far faster still. A Rust rewrite captured most of the asm win. Conclusion: work in Rust source, not assembly.
- **The kernels picked by hand were not hot spots.** A tool must start from a profile of a real workload.
- **The round 1 profile points at design-level costs, not codegen.** Shares are per benchmark:
  - `decode`: about 55% in `CqlType::parse_with_depth` and the string compares it drives (type strings appear to be re-parsed per value).
  - `partition_lookup`: about 22% in SipHash (default `HashMap` hasher) and 20% in `bcmp`.
  - `read`: about 31% allocator, 7% memcpy, about 10% `Arc<str>` clone and drop, and 3% `Value::into_owned`.
  
  These are hypotheses until your fresh profile confirms them.
- **The referee matters most.**
  - Use an A/A null run to size the noise band. The win threshold is T = max(5%, 2 × band).
  - Use a noise gate that refuses to issue a verdict on a noisy run.
  - Report bootstrap intervals over sessions.
  - Give verdicts as WON / TIED / LOST.
  - Claims smaller than T are not made.
  
  The round 1 harness code is in `experiment/harness/src/{stats,measure}.rs` (bootstrap seed 0x6D7378, per-run nonce). Reuse the statistics and the gate logic. Do not reuse the C-ABI kernel loader: round 2 measures whole benchmarks.
- **This host is shared and noisy.** Other agents run bursty parallel cargo builds (load averages of 30+ were seen). There is no cpuset isolation. In round 1, three of four `vint` final runs failed the noise gate. Gate every measurement on a quiet host: no rustc or cargo processes and 1-minute load below 2.0, both held for 10 minutes. Record the load and process state at the start of every run.

## Your goal and deliverables

1. **A fresh profile of current CQLite** for the `read`, `decode` and `partition_lookup` workloads, run long enough to be stable (state the duration and the sample counts). Rank the hot spots by share. Add any other CQLite bench whose user-visible operation matters. Ask the owner which ones.
2. **For each hot spot you take on, in rank order:**
   - **Before touching code,** write the linter rule that would have flagged it. Give it a name, the pattern, the evidence that it is hot, and the predicted gain with its reasoning.
   - Make a source-level Rust change on a branch of the round 2 clone, one hot spot per branch.
   - **Referee it:**
     - CQLite's full test suite must pass.
     - On CQLite's own benchmark for that operation, measured end to end, the change must WIN per the protocol above, against the unchanged baseline built the same way.
     - Report the null band, T, the ratio, the 95% interval and the verdict.
     - Also report effects on the other two workloads, so a regression elsewhere is visible.
   - Record whether the rule's prediction held.
3. **A results write-up** in `docs/results/` in the style of round 1:
   - every number traceable to a recorded run;
   - correctness described as "tested by CQLite's suite only";
   - no claim smaller than T;
   - include what did not win and why.
4. **A rule catalogue** at `docs/rules/` with one entry per rule. Each entry gives the pattern, how to detect it (from source, MIR or machine code), the evidence from this round, and whether it paid off. This is the seed of the toolkit.

## Hard constraints

- **Never modify `/home/dev/projects/cqlite`.** Clone it to `work/cqlite-r2` (machsmith's `work/` is gitignored) at the CQLite repo's current HEAD, record that commit, and branch per hot spot there. Benchmarks need `CQLITE_DATASETS_ROOT=/home/dev/projects/cqlite/test-data/datasets`, because a clone lacks the fetched SSTables.
- **Never build into the shared `CARGO_TARGET_DIR`** (`/home/dev/.cache-dev/cargo-target`). Source `experiment/env.sh` and use `msx_cargo <name> ...` so builds go to `work/target/<name>`. Use toolchain 1.97.1.
- **Leave round 1's artifacts unchanged:**
  - `experiment/results/`
  - `work/target/tool/release/msx` (sha256 9007b7ac…)
  - `docs/results/2026-11-llm-asm-experiment.md`
  - the round 1 clone at `work/cqlite`

  Put round 2 code under a new top-level directory, for example `round2/`.
- **Changes stay internal to CQLite** (controller default, not yet confirmed by the owner). Do not change its public API, on-disk format or wire behaviour. If the best fix needs such a change, write it up as a proposal with the evidence, and do not make it.
- **No system package installs, no pushes, no merges into CQLite,** and no pull requests. Patches stay on local branches for the owner to review. You may commit to machsmith's `main` locally. Ask before pushing it.
- **Keep host load light** while anything else is measuring. Never run your own builds during your own measurement windows.

## How to work

- **Start with a design, not code.** Talk with the owner first and settle the open decisions, one question at a time (the owner prefers that). Then write the design to `docs/specs/<date>-round-2-design.md`. Open decisions include:
  - which workloads count;
  - how many hot spots to attempt;
  - whether to reuse CQLite's `perf-gate.json` thresholds or round 1's T;
  - whether a public-API change may ever be made.
- **After the owner approves the design,** write a plan to `docs/plans/<date>-round-2/` and execute it with fresh subagents per task, a review after each task, and a final review on the most capable model.
- **Keep a ledger of decisions:** for each, the decision, why, and what it costs if wrong. Measurement records are append-only, as in round 1.
- **Stop and ask the owner** before:
  - anything irreversible;
  - pushing or publishing;
  - a public-API change;
  - changing a measurement rule after seeing results.

  Round 1's one serious process error was raising a rerun cap mid-run without approval. Do not repeat it: if the protocol needs to change, stop and ask before running again.
- Say plainly when something failed or was skipped. Do not call any result optimal. Do not extrapolate a benchmark win to "CQLite is X% faster" beyond the operation measured.

When you finish, tell the owner in a few sentences what got faster, by how much (with the ledger references), which rules held, and what is left.
