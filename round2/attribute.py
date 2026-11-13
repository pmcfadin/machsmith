#!/usr/bin/env python3
"""Round 2 attribution: bucket perf samples of a CQLite bench, per case and per workload.

Usage: attribute.py <dir> <bench-binary> <out.json>

<dir> is a profile.sh output directory (`$MSX_WORK/r2-profile/<tag>/<workload>`) holding one
`<case-slug>.script` per case (plus `.mmap` and `cases.json`). The scripts come from
`perf script --no-demangle -F ip,sym,symoff,dso`. A directory recorded with `--call-graph dwarf`
(host.json `callgraph: true`, or a path component ending `-cg`) also gets inclusive call-tree
shares.

Output (all shares are percentages of the samples considered):
  samples, per_case {case: {bucket: share}}, workload {bucket: share} (sample-weighted over cases),
  inclusive {frame: share} (cg only), top_self [...]
Flat shares attribute each sample to its leaf frame (inline-resolved), exactly as round 1 did.
Kernel-leaf samples that leak into a :u profile are dropped, as in round 1.
"""
import bisect
import collections
import ctypes
import json
import os
import re
import subprocess
import sys

CQLITE_COMMIT = "7f9abe3663254724439dbe71aff77bb93faa1c2d"
BUCKETS = ["own_decoding", "allocator", "memcpy", "decompression", "checksum",
           "sync", "other_cqlite", "bench_harness", "other"]

OWN_SSTABLE = ("reader::parsing", "index_reader", "bti", "summary_reader",
               "row_cell_state_machine")
HARNESS_RE = re.compile(r"rustc_demangle|pprof|backtrace|gimli|addr2line")
ALLOC_RE = re.compile(
    r"^(__rust_(alloc|dealloc|realloc|alloc_zeroed)|__rdl_|_?_?libc_(malloc|free|calloc|realloc)"
    r"|malloc|free|calloc|realloc|posix_memalign|aligned_alloc|memalign|cfree"
    r"|libc_malloc_internal|_?_?int_(malloc|free|realloc)|_int_|malloc_consolidate|unlink_chunk|tcache_|sysmalloc"
    r"|alloc::alloc::|alloc::raw_vec::|alloc::vec::Vec<.*>::(reserve|grow)|<alloc::alloc::Global)")
# Copies and fills only. Comparisons (memcmp, bcmp) and scans are not "memory copy".
MEM_RE = re.compile(r"^_*(memmove|memcpy|memset|mempcpy)|^__(memmove|memcpy|memset|mempcpy)_")
SYNC_RE = re.compile(r"crossbeam|std::sync::mpmc|std::sync::mpsc|std::sync::(Mutex|RwLock|Condvar)"
                     r"|parking_lot|futex|pthread_(mutex|cond|rwlock)|__lll_lock|std::thread::park|std::sys::sync")
DECOMP_RE = re.compile(r"lz4_flex|^ZSTD_|^HUF_|^FSE_|^ZSTD|zstd::|snap::|miniz|flate2|lz4::|^LZ4_|inflate")
CKSUM_RE = re.compile(r"crc32fast|crc32c|adler|xxhash|xxh|twox_hash|^crc32")


def classify(function, dso):
    """Return the bucket for a function (after inline resolution) in a dso."""
    base = os.path.basename(dso or "")
    if base == "libgcc_s.so.1" or HARNESS_RE.search(function) or function.startswith("criterion::") \
            or "criterion::" in function.split("<")[0]:
        return "bench_harness"
    if DECOMP_RE.search(function):
        return "decompression"
    if CKSUM_RE.search(function):
        return "checksum"
    if SYNC_RE.search(function):
        return "sync"
    if MEM_RE.search(function):
        return "memcpy"
    if ALLOC_RE.search(function):
        return "allocator"
    pre = "cqlite_core::"
    if function.startswith(pre):
        rest = function[len(pre):]
        if rest.startswith("parser::"):
            return "own_decoding"
        ss = "storage::sstable::"
        if rest.startswith(ss):
            tail = rest[len(ss):]
            if any(tail == p or tail.startswith(p + "::") for p in OWN_SSTABLE):
                return "own_decoding"
        return "other_cqlite"
    if function.startswith("<cqlite_core::"):
        return classify(function[1:], dso)
    if function.startswith("cqlite"):
        return "other_cqlite"
    return "other"



MALLOC_FAMILY = ("malloc", "free", "calloc", "realloc", "memalign", "posix_memalign", "aligned_alloc",
                 "valloc", "pvalloc", "malloc_usable_size", "mallinfo", "mallinfo2", "malloc_trim",
                 "__libc_malloc", "__libc_free", "__libc_calloc", "__libc_realloc", "cfree")


class StrippedLib:
    """Names for addresses in a stripped shared object (libc has no .symtab).

    Sources, in order: dynamic symbols; IFUNC targets as resolved on this CPU
    (via dlsym in this process); the remaining function boundaries from the
    .eh_frame FDE ranges. Unnamed functions inside the span of the malloc
    family's exported symbols are named libc_malloc_internal_<addr>, which is
    where glibc's static _int_malloc/_int_free/tcache code lives.
    """

    def __init__(self, path):
        self.path = os.path.realpath(path)
        llvm = os.environ.get("MSX_LLVM_BIN", "")
        nm = os.path.join(llvm, "llvm-nm") if llvm else "llvm-nm"
        out = subprocess.run([nm, "-D", "--defined-only", self.path],
                             capture_output=True, text=True, check=True).stdout
        self.dyn = {}
        ifuncs = {}
        for line in out.splitlines():
            parts = line.split()
            if len(parts) != 3:
                continue
            addr, kind, name = int(parts[0], 16), parts[1], parts[2].split("@")[0]
            if kind in "iI":
                ifuncs[name] = addr
            else:
                self.dyn.setdefault(addr, name)
        self.dyn_by_name = {}
        for a, n in self.dyn.items():
            self.dyn_by_name.setdefault(n, a)
        self.names = dict(self.dyn)
        # IFUNC targets as resolved on this CPU
        try:
            lib = ctypes.CDLL(self.path)
            base = None
            with open("/proc/self/maps") as fh:
                for line in fh:
                    if os.path.realpath(line.split()[-1]) == self.path if len(line.split()) >= 6 else False:
                        a = line.split()
                        base = int(a[0].split("-")[0], 16) - int(a[2], 16)
                        break
            if base is not None:
                for name in ifuncs:
                    try:
                        tgt = ctypes.cast(getattr(lib, name), ctypes.c_void_p).value - base
                    except (AttributeError, TypeError):
                        continue
                    if tgt not in self.names or self.names[tgt] in ifuncs:
                        self.names[tgt] = "__%s_ifunc_target" % name.strip("_")
        except OSError:
            pass
        # function boundaries from FDEs
        out = subprocess.run(["readelf", "--debug-dump=frames", self.path],
                             capture_output=True, text=True).stdout
        starts = []
        for m in re.finditer(r"FDE cie=\S+ pc=([0-9a-f]+)\.\.([0-9a-f]+)", out):
            starts.append((int(m.group(1), 16), int(m.group(2), 16)))
        self.fdes = sorted(set(starts))
        self.fde_starts = [a for a, _ in self.fdes]
        fam = [self.dyn_by_name[n] for n in MALLOC_FAMILY if n in self.dyn_by_name]
        self.malloc_span = (min(fam) - 0x4000, max(fam) + 0x1000) if fam else (0, 0)

    def runtime_base(self, samples, mmap_file=None):
        """Load bias: from perf's MMAP2 events if given, else from a sample perf named."""
        if mmap_file and os.path.exists(mmap_file):
            with open(mmap_file) as fh:
                for line in fh:
                    m = re.search(r"\[0x([0-9a-f]+)\([0-9a-fx]+\) @ (?:0x)?([0-9a-f]+) ", line)
                    if m and os.path.realpath(line.split()[-1]) == self.path:
                        return int(m.group(1), 16) - int(m.group(2), 16)
        for ip, sym, off, dso in samples:
            if os.path.realpath(dso) == self.path and sym in self.dyn_by_name:
                return ip - off - self.dyn_by_name[sym]
        return None

    def name(self, file_addr):
        i = bisect.bisect_right(self.fde_starts, file_addr) - 1
        if i < 0 or file_addr >= self.fdes[i][1]:
            return "libc_unmapped_%x" % file_addr
        start = self.fdes[i][0]
        if start in self.names:
            return self.names[start]
        if self.malloc_span[0] <= start <= self.malloc_span[1]:
            return "libc_malloc_internal_%x" % start
        return "libc_func_%x" % start


KERNEL = 0xFFFF800000000000
LINE_RE = re.compile(r"^\s*([0-9a-fA-F]+)\s+(.*?)\s+\(([^)]*)\)\s*$")
SYM_RE = re.compile(r"^(.*)\+0x([0-9a-fA-F]+)$")


def parse_line(line):
    m = LINE_RE.match(line)
    if not m:
        return None
    ip, symoff, dso = int(m.group(1), 16), m.group(2), m.group(3)
    if dso == "inlined":
        # perf expands inlined frames in call-graph output; attribute.py resolves inlines itself
        # (addr2line), identically for flat and call-graph profiles, so these lines are dropped.
        return None
    sym, off = symoff, 0
    sm = SYM_RE.match(symoff)
    if sm:
        sym, off = sm.group(1), int(sm.group(2), 16)
    return ip, sym, off, dso


def parse_blocks(path, callgraph=True):
    """Yield one sample at a time as a list of frames (leaf first), each (ip, sym, symoff, dso).

    With callgraph, a sample is a run of non-blank lines (perf separates samples with blank lines).
    Without, every line is a sample. A sample whose leaf is a kernel address is dropped
    (a :u profile should have none; round 1 dropped them too); other kernel frames are removed.
    """
    cur = []

    def done():
        nonlocal cur
        block, cur = cur, []
        if block and block[0][0] < KERNEL:
            return [f for f in block if f[0] < KERNEL]
        return None

    with open(path, errors="replace") as fh:
        for line in fh:
            if not line.strip():
                b = done()
                if b:
                    yield b
                continue
            f = parse_line(line)
            if f is None:
                continue
            if callgraph:
                cur.append(f)
            else:
                cur = [f]
                b = done()
                if b:
                    yield b
    b = done()
    if b:
        yield b


def parse_script(path):
    """Round 1 interface: (ip, sym, symoff, dso) per flat sample line, kernel addresses skipped."""
    for b in parse_blocks(path, callgraph=False):
        yield b[0]


def nm_table(binary):
    """symbol name -> list of file addresses, from llvm-nm."""
    llvm = os.environ.get("MSX_LLVM_BIN", "")
    nm = os.path.join(llvm, "llvm-nm") if llvm else "llvm-nm"
    out = subprocess.run([nm, "--no-demangle", "--defined-only", binary],
                         capture_output=True, text=True, check=True).stdout
    table = collections.defaultdict(list)
    for line in out.splitlines():
        parts = line.split(None, 2)
        if len(parts) == 3 and parts[1] in "tTwWiI":
            table[parts[2]].append(int(parts[0], 16))
    return table


def resolve_inline(binary, addrs):
    """addr -> [(function, source) innermost first] via addr2line -i -f -C."""
    addrs = sorted(addrs)
    result = {}
    if not addrs:
        return result
    inp = "\n".join("0x%x" % a for a in addrs) + "\n"
    out = subprocess.run(["addr2line", "-i", "-f", "-C", "-a", "-e", binary],
                         input=inp, capture_output=True, text=True, check=True).stdout
    cur, frames = None, []

    def flush():
        if cur is not None:
            result[cur] = frames[:]

    lines = out.splitlines()
    i = 0
    while i < len(lines):
        ln = lines[i]
        if ln.startswith("0x") and re.fullmatch(r"0x[0-9a-f]+", ln):
            flush()
            cur, frames = int(ln, 16), []
            i += 1
        else:
            if i + 1 < len(lines):
                frames.append((ln, lines[i + 1]))
            i += 2
    flush()
    return result


def strip_hash(name):
    return re.sub(r"::h[0-9a-f]{16}$", "", name)


DISAMBIG_RE = re.compile(r"\[[0-9a-f]{1,16}\]")


def demangle(names):
    """Rust (v0 or legacy) mangled names -> readable, with crate disambiguators removed."""
    names = list(names)
    if not names:
        return {}
    out = subprocess.run(["c++filt", "-s", "rust"], input="\n".join(names) + "\n",
                         capture_output=True, text=True).stdout.splitlines()
    if len(out) != len(names):
        return {n: n for n in names}
    return {n: strip_hash(DISAMBIG_RE.sub("", d)) for n, d in zip(names, out)}


# ---- inclusive (call-tree) attribution -------------------------------------------------------
ARC_STR_RE = re.compile(r"Arc<str[,>]")
ARC_OP_RE = re.compile(r"clone|drop_slow|drop_in_place|drop")
CMP_RE = re.compile(r"(?<![A-Za-z0-9])(bcmp|memcmp)(?![A-Za-z0-9])")
INTO_OWNED_RE = re.compile(r"Value(<[^>]*>)?::into_owned")

NAMED_FRAMES = [
    ("allocator", lambda n, d: classify(n, d) == "allocator"),
    ("core::hash::sip", lambda n, d: "core::hash::sip" in n),
    ("bcmp/memcmp", lambda n, d: bool(CMP_RE.search(n))),
    ("CqlType::parse_with_depth", lambda n, d: "CqlType::parse_with_depth" in n),
    ("Arc<str> clone/drop", lambda n, d: bool(ARC_STR_RE.search(n)) and bool(ARC_OP_RE.search(n))),
    ("Value::into_owned", lambda n, d: bool(INTO_OWNED_RE.search(n))),
]
TOP_INCLUSIVE = 30


def is_cqlite(name):
    return name.startswith("cqlite_core::") or name.startswith("<cqlite_core::")


def inclusive_counts(chains):
    """chains: iterable of samples, each a list of (name, dso) over all frames (inlined ones included).
    Returns (n_samples, Counter). A named frame, or a CQLite function, counts at most once per sample.
    Samples that run inside the in-process profiler (pprof, backtrace, gimli, rustc_demangle) are
    harness work: they stay in the denominator but credit no frame."""
    n = 0
    hits = collections.Counter()
    for chain in chains:
        n += 1
        if any(HARNESS_RE.search(name) for name, _ in chain):
            continue
        seen = set()
        for name, dso in chain:
            for key, pred in NAMED_FRAMES:
                if key not in seen and pred(name, dso):
                    seen.add(key)
            if is_cqlite(name):
                seen.add("cqlite:" + name)
        hits.update(seen)
    return n, hits


def inclusive_report(n, hits):
    """Percent shares: every named frame (0.0 if absent) plus the top 30 inclusive CQLite functions."""
    out = {key: 100.0 * hits.get(key, 0) / n if n else 0.0 for key, _ in NAMED_FRAMES}
    cq = sorted(((k, c) for k, c in hits.items() if k.startswith("cqlite:")), key=lambda kv: (-kv[1], kv[0]))
    for k, c in cq[:TOP_INCLUSIVE]:
        out[k] = 100.0 * c / n if n else 0.0
    return out


def inclusive_shares(chains):
    return inclusive_report(*inclusive_counts(chains))


# ---- combining cases -------------------------------------------------------------------------
def combine(per_case):
    """per_case: {case: (n_samples, {bucket: count})} -> samples, per_case shares, workload shares (percent).
    The workload share is the sample-weighted mean over cases, i.e. total bucket count / total samples."""
    total = sum(n for n, _ in per_case.values())
    shares = {c: {b: 100.0 * cnt.get(b, 0) / n if n else 0.0 for b in BUCKETS} for c, (n, cnt) in per_case.items()}
    workload = {b: 100.0 * sum(cnt.get(b, 0) for _, cnt in per_case.values()) / total if total else 0.0
                for b in BUCKETS}
    return {"samples": total, "per_case": shares, "workload": workload}


class Context:
    def __init__(self, binary):
        self.binary = binary
        self.real = os.path.realpath(binary)
        self.nm = nm_table(binary)
        self.libs = {}
        self.inline = {}
        self.demangled = {}

    def is_bench(self, dso):
        return os.path.realpath(dso) == self.real if dso.startswith("/") else False

    def lib(self, dso):
        key = os.path.realpath(dso)
        if key not in self.libs:
            self.libs[key] = StrippedLib(dso)
        return self.libs[key]

    def demangle_more(self, names):
        todo = [n for n in set(names) if n not in self.demangled]
        self.demangled.update(demangle(todo))


def frame_addr(ctx, frame, leaf):
    """File address of a bench frame (nm symbol address + offset; a caller's return address minus one)."""
    ip, sym, off, dso = frame
    addrs = ctx.nm.get(sym)
    if not addrs or len(set(addrs)) != 1:
        return None
    return addrs[0] + off - (0 if leaf or off == 0 else 1)


def process_script(ctx, script, callgraph):
    """-> (n, bucket Counter, func_stats {(fn,bucket): {...}}, chains or None)."""
    samples = list(parse_blocks(script, callgraph))
    flat = [f for s in samples for f in s]
    mmap = os.path.splitext(script)[0] + ".mmap"
    base = {}
    for _, _, _, dso in flat:
        if dso.endswith("libc.so.6") and dso not in base:
            base[dso] = ctx.lib(dso).runtime_base(flat, mmap)
    need = set()
    for s in samples:
        for i, f in enumerate(s if callgraph else s[:1]):
            if ctx.is_bench(f[3]):
                a = frame_addr(ctx, f, i == 0)
                if a is not None:
                    need.add(a)
    need -= set(ctx.inline)
    ctx.inline.update(resolve_inline(ctx.binary, need))
    for a in need:
        ctx.inline.setdefault(a, [])

    def names_of(f, leaf):
        """Function names for a frame, innermost inline first; [0] is the flat-attribution function."""
        ip, sym, off, dso = f
        if ctx.is_bench(dso):
            a = frame_addr(ctx, f, leaf)
            frames = ctx.inline.get(a) if a is not None else None
            if frames and frames[0][0] != "??":
                return [fn for fn, _ in frames], frames[0][1].rsplit(":", 1)[0], strip_hash(sym)
            return [strip_hash(sym)], "", strip_hash(sym)
        phys = strip_hash(sym)
        if sym == "[unknown]" and dso.endswith("libc.so.6") and base.get(dso) is not None:
            phys = ctx.lib(dso).name(ip - (0 if leaf else 1) - base[dso])
        return [phys], "", phys

    total = collections.Counter()
    stats = {}
    chains = [] if callgraph else None
    for s in samples:
        names, src, phys = names_of(s[0], True)
        fn = names[0]
        d = ctx.binary if ctx.is_bench(s[0][3]) else s[0][3]
        b = classify(fn, d)
        total[b] += 1
        st = stats.setdefault((fn, b), {"count": 0, "src": src, "phys": collections.Counter()})
        st["count"] += 1
        st["phys"][phys] += 1
        if callgraph:
            chain = []
            for i, f in enumerate(s):
                ns, _, ph = names_of(f, i == 0)
                chain.extend((x, f[3]) for x in ns + [ph])
            chains.append(chain)
    if callgraph:
        # physical symbols are mangled (v0 keeps type arguments, so Arc<str> is visible); demangle them
        mangled = [x for c in chains for x, _ in c if x.startswith(("_R", "_ZN"))]
        ctx.demangle_more(mangled)
        chains = [[(ctx.demangled.get(x, x), d) for x, d in c] for c in chains]
    return len(samples), total, stats, chains


def read_cases(dirpath, slugs):
    try:
        with open(os.path.join(dirpath, "cases.json")) as fh:
            m = json.load(fh)
    except (OSError, ValueError):
        m = {}
    return {s: m.get(s, s) for s in slugs}


def detect_callgraph(dirpath):
    try:
        with open(os.path.join(dirpath, "host.json")) as fh:
            return bool(json.load(fh).get("callgraph"))
    except (OSError, ValueError):
        parts = os.path.abspath(dirpath).split(os.sep)
        return any(p.endswith("-cg") for p in parts)


def entry(fn, b, st, n):
    return {"function": fn, "source_file": st["src"], "bucket": b, "share": 100.0 * st["count"] / n,
            "physical_symbols": [x for x, _ in st["phys"].most_common(5)]}


def main(argv):
    dirpath, binary, out_path = argv[1:4]
    callgraph = detect_callgraph(dirpath)
    slugs = sorted(f[:-len(".script")] for f in os.listdir(dirpath) if f.endswith(".script"))
    if not slugs:
        print("attribute: no .script files in %s" % dirpath, file=sys.stderr)
        return 1
    ids = read_cases(dirpath, slugs)
    ctx = Context(binary)

    per = {}
    func_stats = {}
    incl_all = collections.Counter()
    incl_n = 0
    per_case_top = {}
    incl_case = {}
    for slug in slugs:
        n, total, stats, chains = process_script(ctx, os.path.join(dirpath, slug + ".script"), callgraph)
        case = ids[slug]
        per[case] = (n, total)
        for k, st in stats.items():
            g = func_stats.setdefault(k, {"count": 0, "src": st["src"], "phys": collections.Counter()})
            g["count"] += st["count"]
            g["phys"].update(st["phys"])
        per_case_top[case] = [entry(fn, b, st, n) for (fn, b), st in
                              sorted(stats.items(), key=lambda kv: -kv[1]["count"])[:10]] if n else []
        if callgraph:
            cn, hits = inclusive_counts(chains)
            incl_case[case] = inclusive_report(cn, hits)
            incl_n += cn
            incl_all.update(hits)
        print("attribute: %s: %d samples" % (slug, n), file=sys.stderr)

    result = combine(per)
    n = result["samples"]
    ranked = sorted(func_stats.items(), key=lambda kv: -kv[1]["count"])
    result.update({
        "workload_dir": os.path.abspath(dirpath),
        "cqlite_commit": CQLITE_COMMIT,
        "callgraph": callgraph,
        "case_samples": {c: v[0] for c, v in per.items()},
        "top_self": [entry(fn, b, st, n) for (fn, b), st in ranked[:40]] if n else [],
        # additive: the report lists the top 15 own_decoding functions
        "own_decoding_top": [entry(fn, b, st, n) for (fn, b), st in ranked if b == "own_decoding"][:15] if n else [],
        "per_case_top_self": per_case_top,
    })
    if callgraph:
        result["inclusive"] = inclusive_report(incl_n, incl_all)
        result["inclusive_per_case"] = incl_case
    text = json.dumps(result, indent=2) + "\n"
    with open(out_path, "w") as fh:
        fh.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
