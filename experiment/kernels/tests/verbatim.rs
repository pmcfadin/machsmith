//! Every `BEGIN VERBATIM <file> :: <item> @ <commit>` block in src/ must appear,
//! after normalisation, in that file of the pinned CQLite clone.
use std::path::{Path, PathBuf};

fn normalize(text: &str) -> String {
    let mut kept = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if ["debug!", "trace!", "info!", "warn!", "tracing::"].iter().any(|p| t.starts_with(p)) {
            continue;
        }
        kept.push(line);
    }
    kept.join("\n")
        .split_whitespace()
        .filter(|t| !matches!(*t, "pub" | "pub(crate)" | "pub(super)"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn blocks(src: &str) -> Vec<(String, String)> {
    let mut res = Vec::new();
    let mut cur: Option<(String, String)> = None;
    for line in src.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("// BEGIN VERBATIM ") {
            let file = rest.split_whitespace().next().unwrap().to_string();
            cur = Some((file, String::new()));
        } else if t == "// END VERBATIM" {
            res.push(cur.take().expect("END without BEGIN"));
        } else if let Some((_, body)) = cur.as_mut() {
            body.push_str(line);
            body.push('\n');
        }
    }
    assert!(cur.is_none(), "unterminated VERBATIM block");
    res
}

#[test]
fn verbatim_blocks_match_cqlite() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/cqlite");
    if !root.exists() {
        eprintln!("SKIP verbatim: {} is absent", root.display());
        return;
    }
    let mut total = 0;
    for f in ["vint.rs", "index_framing.rs", "bti_sparse_find.rs"] {
        let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(f)).unwrap();
        for (file, body) in blocks(&src) {
            let theirs = std::fs::read_to_string(root.join(&file)).unwrap();
            let (n, t) = (normalize(&body), normalize(&theirs));
            assert!(!n.is_empty());
            assert!(t.contains(&n), "block from {f} not found in {file}:\n{n}");
            total += 1;
        }
    }
    assert!(total >= 5, "expected at least 5 verbatim blocks, saw {total}");
}
