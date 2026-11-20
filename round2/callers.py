#!/usr/bin/env python3
"""Round 2 caller attribution: credit allocator and memcpy samples to their call site in the bench binary.

Usage: callers.py <dir> <bench-binary> <out.json>

<dir> is a branch-mode profile.sh directory (`$MSX_WORK/r2-profile/<tag>-br/<workload>`, recorded with
`perf record -e cycles:u -j any_call,u`) holding one `<case-slug>.script` per case (plus `.mmap` and
`cases.json`). The scripts come from `perf script --no-demangle -F ip,sym,symoff,dso,brstack`: one line
per sample, the leaf frame followed by the branch stack (most recent call first).

Each sample's leaf is bucketed exactly as attribute.py does it. For an allocator or memcpy sample the
branch stack (any_call: calls only, most recent first) is walked from the newest entry:
  - a call from libc or another non-bench dso into a non-bench dso (libc calling its own internals),
    or a call from bench allocator/memcpy glue (the __rdl_alloc -> malloc call, RawVec growth, Vec and
    String drop glue, hashbrown allocation) or from the PLT into libc or glue, is on the way to the leaf:
    the walk goes on to the older entry;
  - the first call from ordinary bench code is the caller, provided it lands in a non-bench dso, the
    PLT or bench glue. Its site is the innermost cqlite_core frame of the inline chain at `from`, else
    the innermost frame that is not glue;
  - any entry that lands in ordinary bench code before a caller is found means the recorded calls do
    not lead to the leaf (sampling skid, or a call that has already returned): inconsistent, not
    credited;
  - a bench `from` that addr2line cannot resolve, or running out of entries: no caller.
A sample with no branch records counts in brstack_empty_pct (branch sampling produced nothing:
infeasible, not "no caller").

Output (percentages; "with records" = the bucket's samples that carry at least one branch record):
  samples                 all (non-kernel-leaf) samples
  bucket_samples          {allocator, memcpy: count}, all of the bucket's samples
  brstack_empty_pct       samples with no branch records / samples
  no_caller_pct           {bucket: samples with no caller / bucket samples with records}
  inconsistent_pct        {bucket: inconsistent samples / bucket samples with records}
  sites                   {bucket: top 25 [{site, share_of_bucket, share_of_workload}]}
                            share_of_bucket   = site samples / bucket samples with records
                            share_of_workload = site samples / samples
  other_sites_pct         {bucket: credited samples outside the top 25 / bucket samples with records}
                          (top 25 + other_sites_pct + no_caller_pct + inconsistent_pct = 100 per bucket)
  per_case_sites          {case: {bucket: top 10 [{site, share_of_bucket, share_of_case}]}}
                            share_of_bucket = over the case's bucket samples with records
                            share_of_case   = site samples / the case's samples
  case_samples, workload_dir, cqlite_commit, branch: true
The output is structural evidence only (DECISIONS R23).
"""
import bisect
import collections
import json
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import attribute  # noqa: E402

CALLER_BUCKETS = ["allocator", "memcpy"]
TOP_SITES = 25
TOP_CASE_SITES = 10

LEAF_RE = re.compile(r"^\s*([0-9a-fA-F]+)\s+(\S.*?)\s+\(([^)]*)\)(.*)$")
# 0xFROM[ (dso)]/0xTO[ (dso)]/flags...; perf annotates each address with its dso when it knows it.
BR_RE = re.compile(r"0x([0-9a-fA-F]+)(?: \([^)]*\))?/0x([0-9a-fA-F]+)(?: \([^)]*\))?/")
MMAP_RE = re.compile(r"\[0x([0-9a-f]+)\(0x([0-9a-f]+)\) @ (?:0x)?([0-9a-f]+) ")
DISCRIM_RE = re.compile(r"\s*\(discriminator \d+\)$")
# Allocation plumbing beyond attribute.ALLOC_RE (which stays as is: it set the A/B shares): the
# global-allocator shim (__rust_alloc -> __rdl_alloc -> System::alloc), RawVec, drop glue of the
# owning containers, Vec growth helpers and hashbrown's table allocation.
GLUE_RE = re.compile(
    r"^<?std::alloc::|^std::sys::alloc::|^(__rustc::)?__r(dl|ust)_"
    r"|^<?alloc::raw_vec::|^<?alloc::alloc::|^alloc::boxed::box_new"
    r"|^core::ptr::(drop_glue|drop_in_place)::<\[?(alloc::raw_vec::RawVec|alloc::vec::Vec|alloc::string::String"
    r"|alloc::boxed::Box)<"
    r"|^core::ptr::(drop_glue|drop_in_place)::<alloc::string::String>$"
    r"|^<alloc::vec::Vec<.*>>::(with_capacity(_in)?|reserve(_exact)?|try_reserve\w*|grow\w*|push\w*"
    r"|extend_reserve|buf_reserve\w*)(::<.*>)?$"
    r"|^hashbrown::alloc::"
    r"|^<hashbrown::raw::RawTableInner>::(new_uninitialized|fallible_with_capacity)")
CREDITED, NO_CALLER, INCONSISTENT = "credited", "no_caller", "inconsistent"
# summarize() record value for an inconsistent sample (a site is a str, no caller is None)
INCONSISTENT_SITE = object()


def parse_brstack(field):
    """(from, to) pairs, most recent first, from perf's brstack tokens (plain or dso-annotated)."""
    return [(int(f, 16), int(t, 16)) for f, t in BR_RE.findall(field)]


def parse_sample(line):
    """One `perf script -F ip,sym,symoff,dso,brstack` line -> ((ip, sym, symoff, dso), brstack) or None."""
    m = LEAF_RE.match(line.rstrip("\n"))
    if not m:
        return None
    ip, symoff, dso, rest = int(m.group(1), 16), m.group(2), m.group(3), m.group(4)
    sym, off = symoff, 0
    sm = attribute.SYM_RE.match(symoff)
    if sm:
        sym, off = sm.group(1), int(sm.group(2), 16)
    return (ip, sym, off, dso), parse_brstack(rest)


def short_source(src):
    """addr2line `file:line` without the discriminator, the path cut to the crate or std library root."""
    src = DISCRIM_RE.sub("", src)
    for marker in ("/cqlite-core/", "/library/", "/.cargo/registry/src/"):
        i = src.find(marker)
        if i >= 0:
            src = src[i + 1:]
            if marker.startswith("/.cargo"):
                src = src.split("/", 4)[-1]
            break
    return src


def clean_function(name):
    return attribute.strip_hash(attribute.DISAMBIG_RE.sub("", name))


def format_site(function, source):
    return "%s (%s)" % (clean_function(function), short_source(source))


def is_glue(function):
    """Allocator or memcpy plumbing: a call made from it says nothing about who asked for the memory."""
    name = clean_function(function)
    return bool(GLUE_RE.search(name)) or attribute.classify(name, "") in CALLER_BUCKETS


def classify_caller(ctx, leaf_dso, brstack):
    """-> (CREDITED, site) | (NO_CALLER, None) | (INCONSISTENT, None); the walk is in the module docstring.

    `leaf_dso` is accepted for the interface; the rule does not use it, since a credited call must land
    in a non-bench dso (libc), the PLT or bench glue whether the leaf is in libc or in bench glue.
    """
    def frames_at(addr):
        return [(fn, src) for fn, src in ctx.inline_frames(ctx.file_addr(addr)) if fn != "??"]

    def bench(addr):
        dso = ctx.dso_of(addr)
        return dso is not None and ctx.is_bench(dso)

    def plt(addr):
        return ctx.file_addr(addr) is not None and ctx.in_plt(ctx.file_addr(addr))

    def lands_ok(to):
        if not bench(to) or plt(to):
            return True
        frames = frames_at(to)
        return bool(frames) and all(is_glue(fn) for fn, _ in frames)

    for frm, to in brstack:
        if not lands_ok(to):
            return INCONSISTENT, None
        if not bench(frm) or plt(frm):
            continue
        if ctx.file_addr(frm) is None:
            return NO_CALLER, None
        frames = frames_at(frm)
        if not frames:
            return NO_CALLER, None
        if all(is_glue(fn) for fn, _ in frames):
            continue
        for fn, src in frames:
            if attribute.is_cqlite(fn):
                return CREDITED, format_site(fn, src)
        return CREDITED, format_site(*next(f for f in frames if not is_glue(f[0])))
    return NO_CALLER, None


def caller_site(ctx, leaf_dso, brstack):
    """Site of the call from the bench binary that led to the leaf, or None (no caller or inconsistent)."""
    return classify_caller(ctx, leaf_dso, brstack)[1]


def record_value(ctx, leaf_dso, brstack):
    """summarize()'s site field: a site, None (no caller) or INCONSISTENT_SITE."""
    status, site = classify_caller(ctx, leaf_dso, brstack)
    return INCONSISTENT_SITE if status == INCONSISTENT else site


def top_sites(counter, bucket_n, total_n, k, total_key="share_of_workload"):
    ranked = sorted(counter.items(), key=lambda kv: (-kv[1], kv[0]))[:k]
    return [{"site": s, "share_of_bucket": 100.0 * c / bucket_n if bucket_n else 0.0,
             total_key: 100.0 * c / total_n if total_n else 0.0} for s, c in ranked]


def pct(a, b):
    return 100.0 * a / b if b else 0.0


def summarize(records):
    """records: iterable of (case, bucket, has_brstack, site), one per sample; site is a str,
    None (no caller) or INCONSISTENT_SITE. Denominators are in the module docstring."""
    n = 0
    empty = 0
    bucket_n = collections.Counter()
    with_br = collections.Counter()
    no_caller = collections.Counter()
    inconsistent = collections.Counter()
    sites = {b: collections.Counter() for b in CALLER_BUCKETS}
    case_n = collections.Counter()
    case_with_br = collections.defaultdict(collections.Counter)
    case_sites = collections.defaultdict(lambda: {b: collections.Counter() for b in CALLER_BUCKETS})
    for case, bucket, has_br, site in records:
        n += 1
        case_n[case] += 1
        if not has_br:
            empty += 1
        if bucket not in CALLER_BUCKETS:
            continue
        bucket_n[bucket] += 1
        if not has_br:
            continue
        with_br[bucket] += 1
        case_with_br[case][bucket] += 1
        if site is None:
            no_caller[bucket] += 1
        elif site is INCONSISTENT_SITE:
            inconsistent[bucket] += 1
        else:
            sites[bucket][site] += 1
            case_sites[case][bucket][site] += 1
    top = {b: top_sites(sites[b], with_br[b], n, TOP_SITES) for b in CALLER_BUCKETS}
    shown = {b: sum(c for _, c in sorted(sites[b].items(), key=lambda kv: (-kv[1], kv[0]))[:TOP_SITES])
             for b in CALLER_BUCKETS}
    return {
        "samples": n,
        "bucket_samples": {b: bucket_n[b] for b in CALLER_BUCKETS},
        "brstack_empty_pct": pct(empty, n),
        "no_caller_pct": {b: pct(no_caller[b], with_br[b]) for b in CALLER_BUCKETS},
        "inconsistent_pct": {b: pct(inconsistent[b], with_br[b]) for b in CALLER_BUCKETS},
        "sites": top,
        "other_sites_pct": {b: pct(sum(sites[b].values()) - shown[b], with_br[b]) for b in CALLER_BUCKETS},
        "per_case_sites": {c: {b: top_sites(case_sites[c][b], case_with_br[c][b], case_n[c], TOP_CASE_SITES,
                                            "share_of_case")
                               for b in CALLER_BUCKETS} for c in case_n},
    }


# ---- address maps ----------------------------------------------------------------------------
def exec_segment_delta(binary):
    """p_vaddr - p_offset of the executable PT_LOAD (maps a file offset to a file address)."""
    out = subprocess.run(["readelf", "-lW", binary], capture_output=True, text=True, check=True).stdout
    for line in out.splitlines():
        p = line.split()
        if len(p) >= 8 and p[0] == "LOAD" and "E" in p[6:-1]:
            return int(p[2], 16) - int(p[1], 16)
    return 0


def plt_ranges(binary):
    out = subprocess.run(["readelf", "-SW", binary], capture_output=True, text=True, check=True).stdout
    ranges = []
    for m in re.finditer(r"\]\s+(\.plt\S*)\s+\S+\s+([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)", out):
        a, size = int(m.group(2), 16), int(m.group(3), 16)
        ranges.append((a, a + size))
    return ranges


def read_maps(mmap_file):
    """[(start, end, pgoff, path)] of the executable mappings perf recorded, sorted by start."""
    maps = []
    try:
        with open(mmap_file) as fh:
            for line in fh:
                m = MMAP_RE.search(line)
                if m:
                    start, size, pgoff = (int(g, 16) for g in m.groups())
                    maps.append((start, start + size, pgoff, line.split()[-1]))
    except OSError:
        pass
    return sorted(maps)


class BranchContext:
    """Address lookups for one script: runtime address -> dso, bench file address, inline chain.

    Wraps an attribute.Context (shared across scripts for its nm table and inline cache); the maps
    differ per script because every run is loaded at a different address (PIE, ASLR).
    """

    def __init__(self, actx, maps, delta, plt):
        self.actx = actx
        self.maps = maps
        self.starts = [m[0] for m in maps]
        self.delta = delta
        self.plt = plt
        self.bias = None
        for start, _end, pgoff, path in maps:
            if actx.is_bench(path):
                self.bias = start - pgoff - delta
                break

    def dso_of(self, addr):
        i = bisect.bisect_right(self.starts, addr) - 1
        if i >= 0 and addr < self.maps[i][1]:
            return self.maps[i][3]
        return None

    def is_bench(self, dso):
        return self.actx.is_bench(dso)

    def file_addr(self, addr):
        return None if self.bias is None else addr - self.bias

    def in_plt(self, file_addr):
        return any(a <= file_addr < b for a, b in self.plt)

    def inline_frames(self, file_addr):
        if file_addr not in self.actx.inline:
            self.prefetch([file_addr])
        return self.actx.inline[file_addr]

    def prefetch(self, file_addrs):
        need = set(file_addrs) - set(self.actx.inline)
        self.actx.inline.update(attribute.resolve_inline(self.actx.binary, need))
        for a in need:
            self.actx.inline.setdefault(a, [])


def leaf_bucket(actx, frame, libc_base):
    """Bucket of a leaf frame, as attribute.process_script does it (inline-resolved in the bench binary,
    stripped-libc names from the load bias)."""
    ip, sym, off, dso = frame
    if actx.is_bench(dso):
        a = attribute.frame_addr(actx, frame, True)
        frames = actx.inline.get(a) if a is not None else None
        fn = frames[0][0] if frames and frames[0][0] != "??" else attribute.strip_hash(sym)
        return attribute.classify(fn, actx.binary)
    fn = attribute.strip_hash(sym)
    if sym == "[unknown]" and dso.endswith("libc.so.6") and libc_base.get(dso) is not None:
        fn = actx.lib(dso).name(ip - libc_base[dso])
    return attribute.classify(fn, dso)


def process_script(actx, script, case, delta, plt):
    """-> list of (case, bucket, has_brstack, site) per sample. Kernel-leaf samples are dropped."""
    samples = []
    with open(script, errors="replace") as fh:
        for line in fh:
            s = parse_sample(line)
            if s and s[0][0] < attribute.KERNEL and s[0][3] != "inlined":
                samples.append(s)
    mmap = os.path.splitext(script)[0] + ".mmap"
    bctx = BranchContext(actx, read_maps(mmap), delta, plt)
    if bctx.bias is None:
        print("callers: WARNING: no mapping of %s in %s; no bench call sites" % (actx.binary, mmap),
              file=sys.stderr)
    flat = [f for f, _ in samples]
    libc_base = {}
    for _, _, _, dso in flat:
        if dso.endswith("libc.so.6") and dso not in libc_base:
            libc_base[dso] = actx.lib(dso).runtime_base(flat, mmap)
    leaf_need = set()
    for f, _ in samples:
        if actx.is_bench(f[3]):
            a = attribute.frame_addr(actx, f, True)
            if a is not None:
                leaf_need.add(a)
    bctx.prefetch(leaf_need)
    buckets = [leaf_bucket(actx, f, libc_base) for f, _ in samples]
    br_need = set()
    if bctx.bias is not None:
        for (f, br), b in zip(samples, buckets):
            if b in CALLER_BUCKETS:
                for pair in br:
                    for addr in pair:
                        d = bctx.dso_of(addr)
                        if d is not None and bctx.is_bench(d):
                            br_need.add(bctx.file_addr(addr))
    bctx.prefetch(br_need)
    records = []
    for (f, br), b in zip(samples, buckets):
        site = record_value(bctx, f[3], br) if b in CALLER_BUCKETS and br else None
        records.append((case, b, bool(br), site))
    return records


def main(argv):
    if len(argv) != 4:
        print("usage: callers.py <dir> <bench-binary> <out.json>", file=sys.stderr)
        return 2
    dirpath, binary, out_path = argv[1:4]
    slugs = sorted(f[:-len(".script")] for f in os.listdir(dirpath) if f.endswith(".script"))
    if not slugs:
        print("callers: no .script files in %s" % dirpath, file=sys.stderr)
        return 1
    ids = attribute.read_cases(dirpath, slugs)
    actx = attribute.Context(binary)
    delta = exec_segment_delta(binary)
    plt = plt_ranges(binary)
    records = []
    case_samples = {}
    for slug in slugs:
        recs = process_script(actx, os.path.join(dirpath, slug + ".script"), ids[slug], delta, plt)
        case_samples[ids[slug]] = len(recs)
        records.extend(recs)
        print("callers: %s: %d samples" % (slug, len(recs)), file=sys.stderr)
    result = summarize(records)
    result.update({
        "workload_dir": os.path.abspath(dirpath),
        "cqlite_commit": attribute.CQLITE_COMMIT,
        "branch": True,
        "case_samples": case_samples,
    })
    if result["samples"] and result["brstack_empty_pct"] > 10.0:
        print("callers: WARNING: %.1f%% of samples carry no branch records" % result["brstack_empty_pct"],
              file=sys.stderr)
    with open(out_path, "w") as fh:
        fh.write(json.dumps(result, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
