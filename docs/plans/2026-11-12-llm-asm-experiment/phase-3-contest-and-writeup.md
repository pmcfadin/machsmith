# Phase 3: ruler check, contest, final exams, write-up (Tasks 11-14)

Read `README.md` in this directory first. This phase builds no new machinery except `msx report`. It runs the experiment, so its "tests" are gates and recorded results.

Throughout: `MSX=/home/dev/projects/machsmith/work/target/tool/release/msx`, run from `/home/dev/projects/machsmith/experiment` after `. env.sh`. The `msx` binary must not be rebuilt between Task 11 and the end of Task 13; every ledger record carries its hash, and the write-up states that one hash covers all results. If a harness bug forces a rebuild, rerun Task 11 from the start and say so in the write-up.

You are the **orchestrator** in this phase. You do not write candidates, and you do not give candidate agents hints beyond the brief.

---

### Task 11: Run the ruler check

**Files:**
- Create: `experiment/results/null/<kernel>-<ledger id>.json` (written by the tool)
- Create: `experiment/results/ruler-check.md`

- [ ] **Step 1: Confirm the starting state**

```bash
git status --porcelain            # expected: empty
$MSX rungs build                  # expected: JSON with four hashes
$MSX selftest                     # expected: "pass": true
```

- [ ] **Step 2: Confirm every `A` build agrees with `A.r1`**

For each kernel and each of `r2`, `r3`, `r3copy`:

```bash
$MSX check --kernel <K> --so ../work/build/A/<rung>/libmsx_kernels.so --phase final --seed-from kernels/CARDS/<K>.md
```

Expected: `"verdict":"PASS"` for all nine (six if a kernel was dropped). A failure means the compiler builds disagree with each other, which would be a finding about the copied code or the build, not about candidates: stop and report.

- [ ] **Step 3: Run the null for each kernel**

```bash
for K in vint index_framing bti_sparse_find; do $MSX null --kernel $K; done   # omit a kernel dropped in Task 4
```

Run nothing else on the machine meanwhile that you control. Each run takes several minutes.

- [ ] **Step 4: Apply the gate**

For each kernel read `band` and `pass`:
- all three pass (`band ≤ 0.05`): continue;
- any fails: wait at least one hour and rerun that kernel's null. Up to three tries per kernel, at different times of day if possible.
- still failing after three tries: **stop and report (gate 1)**. Include the three bands, the sibling-utilization and discard figures from the session metadata, and the 1-minute load averages. Do not start the contest.

- [ ] **Step 5: Write `experiment/results/ruler-check.md`**

A table: kernel, try number, ledger id, each null interval, band, the threshold `T` that band would give, pass or fail. One paragraph on what the band means for which effects this experiment can and cannot see.

- [ ] **Step 6: Commit**

```bash
git add experiment/results && git commit -m "results: ruler check"
```

---

### Task 12: Run the contest

**Files:**
- Create: `experiment/candidates/<kernel>/BRIEF/{CARD.md,A.r1.s,A.r2.s,A.r3.s,baseline.json,RULES.md}`
- Created by agents: `experiment/candidates/<kernel>/<arm>/{attempt-NN.s|attempt-NN.rs,NOTES.md,FINAL.md}`
- Create: `experiment/results/incidents.md` (only if something goes wrong)

**Interfaces:**
- Consumes: `candidate-agent-prompt.md` in this directory; `msx attempt`.
- Produces: up to six `FINAL.md` files, consumed by Task 13.

- [ ] **Step 1: Build each kernel's brief**

For each kernel, in `experiment/candidates/<K>/BRIEF/`:
- `CARD.md`: a copy of `experiment/kernels/CARDS/<K>.md`.
- `A.r1.s`, `A.r2.s`, `A.r3.s`: the disassembly of `msx_<K>` only, from each rung's library, with source lines interleaved (`llvm-objdump -d -S --no-show-raw-insn --disassemble-symbols=msx_<K>`). If `experiment/selftest/reasm/<K>.s` exists, also copy it as `A.r3.assemblable.s`: it is a legal starting point for the assembly arm.
- `baseline.json`: the output of one search-protocol measurement of the three rungs:
  `$MSX measure --kernel <K> --protocol search --split search --contestants A.r1=…,A.r2=…,A.r3=…`
- `RULES.md`: the "Rules" and "The attempt command" sections of `candidate-agent-prompt.md`, with the placeholders filled in for this kernel.

Create empty directories `experiment/candidates/<K>/asm/` and `…/rust/` (each with a `.gitkeep`). Commit:

```bash
git add experiment/candidates && git commit -m "contest: kernel briefs"
```

- [ ] **Step 2: Dispatch the agents**

Six agents, one per kernel and arm. Each must be a **fresh** agent with no access to this conversation: use a new subagent of the general-purpose kind, never a fork of yourself (a fork inherits your context, including the other arm). If no subagent tool is available, run each as a separate headless session.

- Prompt: `candidate-agent-prompt.md` with every `{{placeholder}}` filled in. Send nothing else.
- `{{MODEL}}` is the model id the agent actually runs on; record it exactly.
- `{{AGENT_ID}}` is `<kernel>-<arm>`.
- Run at most two agents at a time: the two arms of one kernel. The `msx` lock serializes their measurements. Finish a kernel before starting the next.
- Do not answer an agent's questions about how to make the kernel faster. You may answer questions about the rules or a tool error. Record every such exchange in `experiment/results/incidents.md`.

- [ ] **Step 3: After each agent finishes, audit it**

```bash
git status --porcelain
```

Expected: changes only under `experiment/candidates/<K>/<arm>/` and to `experiment/results/ledger.jsonl`. Then check, from the ledger:

- the agent's records for `(K, arm)`: at most 30 submissions, at most 10 counted;
- every `attempt-NN` file in the directory has a ledger record with the same SHA-256;
- `FINAL.md` exists and its first line names an attempt whose record is `PASS`. If the agent had no passing attempt, `FINAL.md` should say `FINAL: none`; that arm is a `NO_ENTRY`.

If the agent changed anything outside its directory: restore those paths with `git checkout -- <path>` (or delete untracked files), and describe it in `incidents.md`. Its ledger records stand, because `msx attempt` refuses to run on a dirty tree. If the agent read the other arm's directory, the holdout traces, or another kernel's candidates (visible in its transcript if you have it), record that in `incidents.md` too; the write-up reports it and the result is kept but flagged.

Commit after each kernel:

```bash
git add experiment/candidates experiment/results && git commit -m "contest: <K> attempts"
```

- [ ] **Step 4: Confirm the contest is complete**

All six arms have either a valid `FINAL.md` or a recorded reason for `NO_ENTRY`.

---

### Task 13: Final exams

**Files:**
- Create: `experiment/results/final/<kernel>.json` (written by the tool)

- [ ] **Step 1: Quiet the machine as far as you can**

No candidate agents running. No builds. `git status --porcelain` empty.

- [ ] **Step 2: Run each final exam once**

```bash
for K in vint index_framing bti_sparse_find; do MSX_AGENT_ID=orchestrator MSX_MODEL=<your model id> $MSX final --kernel $K; done
```

Each run performs the final correctness check on each finalist (20,000,000 random cases seeded from the candidate's own hash, plus the holdout trace cases, plus the edge cases), then 30 sessions of 200 blocks on the holdout chunks. Expect tens of minutes per kernel.

- [ ] **Step 3: Handle the outcomes**

| Outcome | What to do |
|---|---|
| verdicts issued | done for that kernel |
| an arm `DISQUALIFIED` | done; it is reported as disqualified with the failing input. No second pick |
| an arm `NO_ENTRY` | done; the other arm and the `A` rungs are still measured |
| `NOISE_GATE_FAILED` | wait at least one hour; rerun with `--rerun-reason "noise gate failed, try N"`. At most three tries. If still failing, that kernel's result is "unmeasurable on this host on this date" |
| tool error | fix nothing in the harness without restarting from Task 11. A transient error (for example the lock timing out) may be rerun with `--rerun-reason` stating it |

- [ ] **Step 4: Commit**

```bash
git add experiment/results && git commit -m "results: final exams"
```

---

### Task 14: Write-up

**Files:**
- Modify: `experiment/harness/src/commands.rs`, `src/bin/msx.rs` (`report`). This is the one allowed rebuild after Task 13; `report` only reads the ledger.
- Create: `docs/results/2026-11-llm-asm-experiment.md`
- Test: `experiment/harness/tests/report.rs`

**Interfaces:**
- Consumes: the ledger, `experiment/results/final/*.json`, `experiment/results/profile/*.json`, `experiment/kernels/SELECTION.md`, `experiment/results/incidents.md`.
- Produces: `msx report`, which prints Markdown to stdout: (1) the ceiling table; (2) per kernel, a verdict table; (3) per kernel and arm, the attempts history; (4) a provenance block (tool hash, `A` hashes, CQLite commit, host, dates, ledger id range). Every row ends with the ledger id it came from.

Verdict table columns: comparison, ratio, 95% interval, verdict, ledger id. Rows: `B.asm` vs `A.r1`, `A.r2`, `A.r3`; `B.rust` vs the same; `B.rust.r2` vs `A.r2`; `B.asm` vs `B.rust`; `A.r2` vs `A.r1`; `A.r3` vs `A.r2`; nulls; boundary tax. Below the table: the null band and `T` used, and per contestant cycles/op, instructions/op, branch misses/op and ns/op.

Attempts history columns: submission, counted, verdict, provisional ratio vs `A.r3` (or the failure kind), ledger id. Label the ratio column "provisional (search phase)".

- [ ] **Step 1: Write the failing test**

`tests/report.rs`: given a synthetic ledger and a synthetic `final/vint.json` (fixtures in the test), `report` output contains the row `| B.asm vs A.r3 | 0.912 | 0.901–0.923 | WON | L000042 |`, labels the attempts column "provisional (search phase)", and prints `NO_ENTRY` for an arm without a finalist. Expected: fails.

- [ ] **Step 2: Implement `msx report`; run the test** — expected: passes.

- [ ] **Step 3: Write `docs/results/2026-11-llm-asm-experiment.md`**

Sections, in this order. Tables are pasted from `msx report` unedited; do not retype numbers.

1. **Answer.** Three to six sentences answering the spec's question (section 2): on each kernel, did LLM-written assembly beat the compiler at its best on this machine, by how much, and did the Rust rewrite get the same win? State the threshold used. If a kernel was unmeasurable or an arm had no entry, say so here.
2. **What was tested.** The kernels and why (from `SELECTION.md`), the contestants, the machine, the dates, the model that wrote the candidates.
3. **Where CQLite's time goes.** The ceiling table, one sentence per workload on what it means for these kernels, and the plain statement that none of the three kernels is a CQLite hot spot and that no row/cell kernel could be isolated (from `SELECTION.md`). Include the observation, made before the contest, that the compiler turns `decode_unsigned`'s byte copy into a call to `memcpy`, with the disassembly.
4. **Results per kernel.** The verdict table; then:
   - how many attempts each agent used and how many failed correctness;
   - whether the kernel was measured with repeating inputs (small-trace mode), and what that means;
   - the boundary tax, in words ("isolating this kernel behind a call costs X%");
   - what the ladder itself was worth (`A.r2` vs `A.r1`, `A.r3` vs `A.r2`).
5. **What the winners did.** For each `WON` against `A.r3`: the candidate's code beside `A.r3`'s, what is different in plain words, and which counters moved (instructions/op, branch misses/op). Classify the knowledge used, with a sentence of justification:
   - (a) pure code generation: the same contract, better instructions;
   - (b) a fact true in CQLite but invisible to the compiler;
   - (c) a property of the workload's data (for example the distribution of varint lengths).
   Then say whether `B.rust` captured the same win. If nothing won, this section explains what the candidates tried instead and why it did not pay, from the agents' `NOTES.md`.
6. **What failed.** Disqualifications, correctness failures by kind across all attempts, and anything in `incidents.md`.
7. **Limitations.** The seven limitations from spec section 10, each restated with this run's specifics (the actual null band, the actual trace sizes, any "small trace" kernel, whether entry alignment was normalized). Add any discovered during the run.
8. **What this suggests for the tool.** Five to ten bullets: patterns a PMD-style analyzer could flag, drawn only from what was observed here; and which harness pieces proved their worth (name the sabotage cases that mattered).
9. **Reproducing this.** The commands, the ledger id range, the tool and `A` hashes.

Rules for the prose: no claim without a ledger id; no search-phase number outside the attempts history; say "tested only" wherever correctness is mentioned; do not call any result optimal; do not claim an end-to-end CQLite speed-up.

- [ ] **Step 4: Check the write-up against the ledger**

For every number in sections 1, 4 and 5, find it in `msx report` output or in the cited ledger record. Fix any that differ. Confirm each of the seven spec limitations appears in section 7.

- [ ] **Step 5: Commit and hand over**

```bash
git add -A && git commit -m "docs: experiment write-up"
```

Tell the owner the write-up's path, the answer in two sentences, and anything in `incidents.md`.
