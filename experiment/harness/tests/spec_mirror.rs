use msx_harness::spec::{kernels_with_traces, load_segment, manifest_segments};
use msx_harness::{all_kernels, kernel, KernelSpec, RawFn};

fn reference(name: &str) -> RawFn {
    match name {
        "vint" => msx_kernels::vint::msx_vint as RawFn,
        "index_framing" => msx_kernels::index_framing::msx_index_framing as RawFn,
        "bti_sparse_find" => msx_kernels::bti_sparse_find::msx_bti_sparse_find as RawFn,
        _ => panic!("unknown kernel {name}"),
    }
}

type ChainFn = unsafe extern "C" fn(*const u8, usize, *mut u64) -> u64;

fn chain_fn(name: &str) -> ChainFn {
    match name {
        "vint" => msx_kernels::vint::msx_vint_chain,
        "index_framing" => msx_kernels::index_framing::msx_index_framing_chain,
        "bti_sparse_find" => msx_kernels::bti_sparse_find::msx_bti_sparse_find_chain,
        _ => panic!("unknown kernel {name}"),
    }
}

unsafe fn kernel_chain(name: &str, seg: &[u8]) -> (u64, u64) {
    let mut sum = 0u64;
    let ops = chain_fn(name)(seg.as_ptr(), seg.len(), &mut sum);
    (ops, sum)
}

#[test]
fn registry_is_complete() {
    let names: Vec<_> = all_kernels().iter().map(|k| k.name()).collect();
    assert_eq!(names, ["vint", "index_framing", "bti_sparse_find"]);
    for k in all_kernels() {
        assert_eq!(kernel(k.name()).unwrap().symbol(), k.symbol());
        assert_eq!(k.chain_symbol(), format!("{}_chain", k.symbol()));
    }
    assert!(kernel("nope").is_none());
    let traced: Vec<_> = kernels_with_traces().iter().map(|k| k.name()).collect();
    assert_eq!(traced, ["vint", "index_framing"]);
}

#[test]
fn chain_matches_kernel_chain_on_search_segments() {
    for k in kernels_with_traces() {
        let a = reference(k.name());
        let segs = manifest_segments(k.name(), Some("search")).unwrap();
        assert!(!segs.is_empty());
        for s in segs {
            let seg = load_segment(&s.path).unwrap();
            assert_eq!(seg.as_ptr() as usize % 8, 0);
            let got = unsafe { k.run_chain(a, &seg) };
            let want = unsafe { kernel_chain(k.name(), &seg) };
            assert_eq!(got, want, "{} {}", k.name(), s.file);
            assert_eq!(got.0, s.ops, "{} {} ops vs manifest", k.name(), s.file);
        }
    }
}

fn check_chunks(k: &dyn KernelSpec, seg: &[u8], ops: u64, min_ops: u64) {
    let a = reference(k.name());
    let chunks = unsafe { k.chunks(a, seg, min_ops) };
    let mut total = 0u64;
    let mut end = 0usize;
    for c in &chunks {
        // chunks are contiguous prefixes of the segment
        assert_eq!(c.as_ptr() as usize, seg.as_ptr() as usize + end);
        end += c.len();
        let (n, _) = unsafe { k.run_chain(a, c) };
        assert!(n >= min_ops);
        total += n;
    }
    let (tail_ops, _) = unsafe { kernel_chain(k.name(), &seg[end..]) };
    assert!(tail_ops < min_ops || chunks.is_empty() && tail_ops == ops);
    assert_eq!(total + tail_ops, ops, "{} min_ops={min_ops}", k.name());
}

#[test]
fn chunks_account_for_every_op() {
    for k in kernels_with_traces() {
        for s in manifest_segments(k.name(), Some("search")).unwrap() {
            let seg = load_segment(&s.path).unwrap();
            for min_ops in [1000, 100, 7, 1] {
                check_chunks(k, &seg, s.ops, min_ops);
            }
        }
    }
}

#[test]
fn trace_cases_yield_exactly_ops() {
    for k in kernels_with_traces() {
        let a = reference(k.name());
        for s in manifest_segments(k.name(), Some("search")).unwrap() {
            let seg = load_segment(&s.path).unwrap();
            let mut n = 0u64;
            unsafe { k.trace_cases(a, &seg, &mut |_, _| n += 1) };
            assert_eq!(n, s.ops, "{} {}", k.name(), s.file);
        }
    }
}

fn collect_edge(k: &dyn KernelSpec) -> Vec<(Vec<u8>, u64)> {
    let mut v = Vec::new();
    k.edge_cases(&mut |b, a| v.push((b.to_vec(), a)));
    v
}

#[test]
fn edge_cases_are_deterministic_with_expected_counts() {
    for k in all_kernels() {
        let one = collect_edge(*k);
        let two = collect_edge(*k);
        assert!(one == two, "{} edge cases differ between calls", k.name());
        match k.name() {
            "vint" => assert_eq!(one.len(), 33_280),
            "bti_sparse_find" => assert_eq!(one.len(), 32 * 256),
            _ => assert!(one.len() > 100_000),
        }
        eprintln!("edge cases {}: {}", k.name(), one.len());
    }
}

#[test]
fn random_cases_are_deterministic_and_respect_contracts() {
    for k in all_kernels() {
        let run = |seed| {
            let mut v = Vec::new();
            k.random_cases(seed, 500, &mut |b, a| v.push((b.to_vec(), a)));
            v
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
        assert_eq!(run(7).len(), 500);
        if k.name() == "bti_sparse_find" {
            for (b, _) in run(7) {
                assert_eq!(b.len() % 16, 0);
                let keys: Vec<u8> = b.chunks(16).map(|t| t[0]).collect();
                assert!(keys.windows(2).all(|w| w[0] < w[1]));
            }
        }
    }
}

/// The spec's `run_case` through the trampoline agrees with a direct call of the reference.
#[test]
fn run_case_matches_direct_reference_call() {
    use msx_harness::guard::GuardBuf;
    for k in all_kernels() {
        let a = reference(k.name());
        let mut cases = Vec::new();
        k.random_cases(1, 300, &mut |b, x| cases.push((b.to_vec(), x)));
        let mut n = 0usize;
        k.edge_cases(&mut |b, x| {
            n += 1;
            if n % 97 == 0 {
                cases.push((b.to_vec(), x));
            }
        });
        let mut g = GuardBuf::new(70_000);
        for (b, x) in cases {
            let p = g.place_end(&b);
            let rec = unsafe { k.run_case(a, p, b.len(), x) }.expect("reference violates the ABI?");
            // Direct call with the same inputs.
            let (ret, out, w) = unsafe { direct(k.name(), p, b.len(), x) };
            assert_eq!((rec.ret, rec.out, rec.out_written), (ret, out, w), "{}", k.name());
        }
    }
}

unsafe fn direct(name: &str, p: *const u8, len: usize, aux: u64) -> (u64, [u64; 3], u64) {
    const S: u64 = 0xDEAD_BEEF_DEAD_BEEF;
    let mut o = [S; 2];
    let ret = match name {
        "vint" => msx_kernels::vint::msx_vint(p, len, o.as_mut_ptr()) as u64,
        "index_framing" => {
            msx_kernels::index_framing::msx_index_framing(p, len, o.as_mut_ptr() as *mut _) as u64
        }
        _ => msx_kernels::bti_sparse_find::msx_bti_sparse_find(p as *const _, len / 16, aux as u32, o.as_mut_ptr()) as u64,
    };
    let words = if name == "index_framing" { 2 } else { 1 };
    let mut out = [0u64; 3];
    let mut changed = false;
    for i in 0..words {
        out[i] = o[i];
        changed |= o[i] != S;
    }
    (ret, out, changed as u64)
}
