use msx_harness::commands::{parse_final_md, tree_is_clean_for};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const OK_TREE: &str = " M experiment/candidates/vint/asm/attempt-03.s\n?? experiment/candidates/vint/asm/NOTES.md\n M experiment/results/ledger.jsonl\n";

#[test]
fn clean_tree_allows_own_candidate_dir_and_ledger() {
    assert!(tree_is_clean_for(OK_TREE, "vint", "asm").is_ok());
}

#[test]
fn tree_with_foreign_changes_is_rejected() {
    let bad = format!("{OK_TREE} M experiment/harness/src/check.rs\n");
    let e = tree_is_clean_for(&bad, "vint", "asm").unwrap_err();
    assert_eq!(e, vec!["experiment/harness/src/check.rs".to_string()]);
    let bad = format!("{OK_TREE}?? experiment/candidates/vint/rust/x.rs\n");
    let e = tree_is_clean_for(&bad, "vint", "asm").unwrap_err();
    assert_eq!(e, vec!["experiment/candidates/vint/rust/x.rs".to_string()]);
}

#[test]
fn tree_check_is_not_fooled_by_prefixes_renames_or_traversal() {
    // sibling directory sharing a prefix
    assert!(tree_is_clean_for("?? experiment/candidates/vint/asm2/x.s\n", "vint", "asm").is_err());
    assert!(tree_is_clean_for("?? experiment/candidates/vint/asm/../rust/x.rs\n", "vint", "asm").is_err());
    // rename out of the candidate dir lists both ends
    let e = tree_is_clean_for("R  experiment/candidates/vint/asm/a.s -> experiment/harness/src/a.s\n", "vint", "asm").unwrap_err();
    assert_eq!(e, vec!["experiment/harness/src/a.s".to_string()]);
    // quoted paths
    assert!(tree_is_clean_for("?? \"experiment/candidates/vint/asm/a b.s\"\n", "vint", "asm").is_ok());
    assert!(tree_is_clean_for("?? \"experiment/harness/a b.s\"\n", "vint", "asm").is_err());
}

#[test]
fn final_md_parsing() {
    assert_eq!(parse_final_md("FINAL: attempt-07\nwhy it is the best"), Some(7));
    assert_eq!(parse_final_md("FINAL: attempt-07"), Some(7));
    assert_eq!(parse_final_md("final: 7"), None);
    assert_eq!(parse_final_md(""), None);
    assert_eq!(parse_final_md("FINAL: attempt-x"), None);
    assert_eq!(parse_final_md("FINAL: attempt-"), None);
    assert_eq!(parse_final_md("FINAL: attempt-07 and attempt-08"), None);
}

// ---- end-to-end, with a throwaway git repo and ledger (no measurement) ----

struct Env {
    root: PathBuf,
    work: PathBuf,
}

fn setup(tag: &str) -> Env {
    let base = std::env::var("MSX_WORK").unwrap_or_else(|_| format!("{}/../../work", env!("CARGO_MANIFEST_DIR")));
    let dir = PathBuf::from(base).join("tmp").join(format!("cmd-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let root = dir.join("repo");
    let work = dir.join("work");
    std::fs::create_dir_all(root.join("experiment/results")).unwrap();
    std::fs::create_dir_all(root.join("experiment/candidates/vint/asm")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(root.join("README"), "x").unwrap();
    for args in [vec!["init", "-q"], vec!["add", "README"], vec!["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "init"]] {
        assert!(Command::new("git").current_dir(&root).args(&args).status().unwrap().success());
    }
    Env { root, work }
}

fn msx(env: &Env, args: &[&str], identity: bool) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_msx"));
    c.current_dir(env.root.join("experiment"))
        .args(args)
        .env("MSX_ROOT", &env.root)
        .env("MSX_WORK", &env.work)
        .env("MSX_LEDGER", env.root.join("experiment/results/ledger.jsonl"))
        .env_remove("MSX_SESSIONS")
        .env_remove("MSX_BLOCKS");
    if identity {
        c.env("MSX_AGENT_ID", "agent-1").env("MSX_MODEL", "model-x");
    } else {
        c.env_remove("MSX_AGENT_ID").env_remove("MSX_MODEL");
    }
    c.output().unwrap()
}

fn ledger(env: &Env) -> Vec<serde_json::Value> {
    match std::fs::read_to_string(env.root.join("experiment/results/ledger.jsonl")) {
        Ok(t) => t.lines().map(|l| serde_json::from_str(l).unwrap()).collect(),
        Err(_) => vec![],
    }
}

fn put(env: &Env, name: &str, text: &str) -> String {
    let rel = format!("experiment/candidates/vint/asm/{name}");
    std::fs::write(env.root.join(&rel), text).unwrap();
    rel
}

fn attempt(env: &Env, file: &str) -> Output {
    // relative to the repo root; the tool resolves --file against its cwd (experiment/)
    let f = Path::new("..").join(file);
    msx(env, &["attempt", "--kernel", "vint", "--arm", "asm", "--file", f.to_str().unwrap()], true)
}

fn stdout_json(o: &Output) -> serde_json::Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("stdout not JSON ({e}): {}", String::from_utf8_lossy(&o.stdout)))
}

#[test]
fn attempt_refuses_without_identity() {
    let env = setup("noid");
    let f = put(&env, "attempt-01.s", ".text\n");
    let f = Path::new("..").join(f);
    let o = msx(&env, &["attempt", "--kernel", "vint", "--arm", "asm", "--file", f.to_str().unwrap()], false);
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(ledger(&env).is_empty());
    assert!(String::from_utf8_lossy(&o.stderr).contains("MSX_AGENT_ID"));
}

#[test]
fn attempt_refuses_under_size_override() {
    let env = setup("override");
    let f = Path::new("..").join(put(&env, "attempt-01.s", ".text\n"));
    let o = Command::new(env!("CARGO_BIN_EXE_msx"))
        .current_dir(env.root.join("experiment"))
        .args(["attempt", "--kernel", "vint", "--arm", "asm", "--file", f.to_str().unwrap()])
        .env("MSX_ROOT", &env.root)
        .env("MSX_WORK", &env.work)
        .env("MSX_LEDGER", env.root.join("experiment/results/ledger.jsonl"))
        .env("MSX_AGENT_ID", "a")
        .env("MSX_MODEL", "m")
        .env("MSX_SESSIONS", "1")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(ledger(&env).is_empty());
}

#[test]
fn dirty_tree_is_a_tool_error_without_record() {
    let env = setup("dirty");
    let f = put(&env, "attempt-01.s", ".text\n");
    std::fs::write(env.root.join("stray.txt"), "x").unwrap();
    let o = attempt(&env, &f);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("stray.txt"));
    assert!(ledger(&env).is_empty());
}

#[test]
fn wrong_name_and_duplicate_hash_are_tool_errors() {
    let env = setup("names");
    // next submission is 01, not 02
    let f = put(&env, "attempt-02.s", ".data\n");
    assert_eq!(attempt(&env, &f).status.code(), Some(2));
    let f = put(&env, "attempt-01.rs", ".data\n");
    assert_eq!(attempt(&env, &f).status.code(), Some(2));
    assert!(ledger(&env).is_empty());
    // a lint failure is recorded; the same bytes again under attempt-02 are a duplicate
    let f = put(&env, "attempt-01.s", ".data\n");
    let o = attempt(&env, &f);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(ledger(&env).len(), 1);
    let f = put(&env, "attempt-02.s", ".data\n");
    let o = attempt(&env, &f);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("duplicate"));
    assert_eq!(ledger(&env).len(), 1);
}

#[test]
fn lint_failure_is_free_and_recorded() {
    let env = setup("lint");
    let f = put(&env, "attempt-01.s", ".data\n");
    let o = attempt(&env, &f);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let out = stdout_json(&o);
    assert_eq!(out["verdict"], "LINT_FAILED");
    assert_eq!(out["submission"], 1);
    assert!(out["counted_attempt"].is_null());
    assert_eq!(out["counted_attempts_left"], 10);
    assert_eq!(out["submissions_left"], 29);
    assert!(out["timing"].is_null());
    assert!(!out["lint_errors"].as_array().unwrap().is_empty());
    let l = ledger(&env);
    assert_eq!(l.len(), 1);
    assert_eq!(l[0]["kind"], "attempt");
    assert_eq!(l[0]["agent_id"], "agent-1");
    assert_eq!(l[0]["model"], "model-x");
    assert_eq!(l[0]["verdict"], "LINT_FAILED");
    assert!(l[0]["counted_attempt"].is_null());
    assert_eq!(l[0]["candidate_path"], "experiment/candidates/vint/asm/attempt-01.s");
    assert_eq!(l[0]["candidate_sha256"].as_str().unwrap().len(), 64);
}

#[test]
fn build_failure_is_free_and_recorded() {
    let env = setup("build");
    let f = put(&env, "attempt-01.s", ".text\n    bogus_mnemonic %rax\n");
    let o = attempt(&env, &f);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let out = stdout_json(&o);
    assert_eq!(out["verdict"], "BUILD_FAILED");
    assert!(out["counted_attempt"].is_null());
    assert!(out["build_stderr"].as_str().unwrap().contains("bogus_mnemonic"));
    let l = ledger(&env);
    assert_eq!(l[0]["build"]["stage"], "assemble");
}

fn seed_ledger(env: &Env, n_free: u32, n_counted: u32) {
    let mut lines = String::new();
    let mut id = 0;
    for i in 0..(n_free + n_counted) {
        id += 1;
        let counted = if i >= n_free { serde_json::json!(i - n_free + 1) } else { serde_json::Value::Null };
        lines += &format!(
            "{}\n",
            serde_json::json!({"id": format!("L{id:06}"), "kind": "attempt", "kernel": "vint", "arm": "asm", "agent_id": "agent-1",
                "submission": i + 1, "counted_attempt": counted, "verdict": "INCORRECT", "candidate_sha256": format!("{i:064}")})
        );
    }
    std::fs::write(env.root.join("experiment/results/ledger.jsonl"), lines).unwrap();
}

#[test]
fn exhausted_counted_budget_is_refused_and_recorded() {
    let env = setup("budget-counted");
    seed_ledger(&env, 2, 10);
    let f = put(&env, "attempt-13.s", ".text\n");
    let o = attempt(&env, &f);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(stdout_json(&o)["verdict"], "BUDGET_EXHAUSTED");
    let l = ledger(&env);
    assert_eq!(l.len(), 13);
    assert_eq!(l[12]["kind"], "refused");
    assert_eq!(l[12]["id"], "L000013");
    assert!(l[12]["counted_attempt"].is_null());
}

#[test]
fn exhausted_submission_budget_is_refused() {
    let env = setup("budget-sub");
    seed_ledger(&env, 30, 0);
    let f = put(&env, "attempt-31.s", ".text\n");
    let o = attempt(&env, &f);
    assert_eq!(stdout_json(&o)["verdict"], "BUDGET_EXHAUSTED");
    assert_eq!(ledger(&env).last().unwrap()["kind"], "refused");
    // refused records do not advance the submission number or the budget
    let f = put(&env, "attempt-32.s", ".data\n");
    assert_eq!(attempt(&env, &f).status.code(), Some(2));
}

#[test]
fn null_and_final_refuse_unknown_or_traceless_kernels() {
    let env = setup("traceless");
    for cmd in ["null", "final"] {
        let o = msx(&env, &[cmd, "--kernel", "bti_sparse_find"], true);
        assert_eq!(o.status.code(), Some(2), "{cmd}");
        let o = msx(&env, &[cmd, "--kernel", "nope"], true);
        assert_eq!(o.status.code(), Some(2), "{cmd}");
    }
    assert!(ledger(&env).is_empty());
}

#[test]
fn final_refuses_a_second_measured_final_without_a_reason() {
    let env = setup("final-twice");
    std::fs::write(
        env.root.join("experiment/results/ledger.jsonl"),
        format!("{}\n", serde_json::json!({"id": "L000001", "kind": "final", "kernel": "vint", "measure": {"x": 1}})),
    )
    .unwrap();
    let o = msx(&env, &["final", "--kernel", "vint"], true);
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("rerun-reason"));
    assert_eq!(ledger(&env).len(), 1);
}

#[test]
fn final_with_no_entries_records_no_entry_without_measuring() {
    let env = setup("final-noentry");
    let o = msx(&env, &["final", "--kernel", "vint", "--rerun-reason", "none, first run"], true);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let out = stdout_json(&o);
    assert_eq!(out["arms"]["asm"]["status"], "NO_ENTRY");
    assert_eq!(out["arms"]["rust"]["status"], "NO_ENTRY");
    let l = ledger(&env);
    assert_eq!(l[0]["kind"], "final");
    assert!(l[0]["measure"].is_null());
    assert_eq!(l[0]["rerun_reason"], "none, first run");
}
