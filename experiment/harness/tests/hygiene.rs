use msx_harness::measure::{apply_hygiene, session_is_valid, Discard, Sample};

fn s(block: u32, chunk: u32, who: &str, instr: u64, d: Option<Discard>) -> Sample {
    Sample {
        session: 0,
        block,
        chunk,
        contestant: who.into(),
        ops: 50_000,
        cycles: 1_000_000,
        instructions: instr,
        branch_misses: 10,
        wall_ns: 300_000,
        discard: d,
    }
}

/// `blocks` blocks, two contestants, one chunk per block (chunk == block % 4), identical instruction counts.
fn clean(blocks: u32) -> Vec<Sample> {
    let mut v = Vec::new();
    for b in 0..blocks {
        for who in ["a", "b"] {
            v.push(s(b, b % 4, who, 1_000_000, None));
        }
    }
    v
}

#[test]
fn preempted_discards_its_whole_block_only() {
    let mut v = clean(12);
    v[2 * 5].discard = Some(Discard::Preempted); // block 5, contestant a
    apply_hygiene(&mut v);
    for x in &v {
        if x.block == 5 {
            assert!(x.discard.is_some(), "block 5 sample kept: {x:?}");
        } else {
            assert!(x.discard.is_none(), "unrelated sample discarded: {x:?}");
        }
    }
    assert_eq!(v[2 * 5].discard, Some(Discard::Preempted));
}

#[test]
fn instruction_count_tolerance() {
    // chunk 0 is visited by blocks 0, 4, 8, 12, 16 for each contestant
    let mut v = clean(20);
    let idx = |block: usize, who: usize| 2 * block + who;
    v[idx(4, 0)].instructions = 1_001_000; // 0.1% off the mode
    v[idx(8, 1)].instructions = 1_000_010; // 0.001% off the mode
    apply_hygiene(&mut v);
    assert_eq!(v[idx(4, 0)].discard, Some(Discard::InstrMismatch));
    assert!(v[idx(4, 1)].discard.is_some(), "the block's other sample is discarded too");
    assert!(v[idx(8, 1)].discard.is_none(), "0.001% must be kept");
    assert!(v[idx(8, 0)].discard.is_none());
    assert!(v[idx(12, 0)].discard.is_none());
}

#[test]
fn no_mode_marks_every_sample_of_that_group() {
    let mut v = Vec::new();
    // contestant a on chunk 0 has four mutually different counts
    for (b, instr) in [(0u32, 1_000_000u64), (1, 1_100_000), (2, 1_200_000), (3, 1_300_000)] {
        v.push(s(b, 0, "a", instr, None));
    }
    apply_hygiene(&mut v);
    assert!(v.iter().all(|x| x.discard == Some(Discard::InstrMismatch)));
}

fn with_discarded_blocks(total: u32, bad: u32) -> Vec<Sample> {
    let mut v = clean(total);
    for b in 0..bad {
        v[2 * b as usize].discard = Some(Discard::Preempted);
    }
    apply_hygiene(&mut v);
    v
}

#[test]
fn session_validity_threshold() {
    assert!(!session_is_valid(&with_discarded_blocks(100, 31)));
    assert!(session_is_valid(&with_discarded_blocks(100, 30)));
}
