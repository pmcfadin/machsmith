#!/usr/bin/env python3
"""Bucket perf samples of a CQLite bench into the ceiling categories.

Usage: ceiling.py <bench> <perf-script-file> <bench-binary> [out.json]

The perf script must come from `perf script -F ip,sym,symoff,dso`
(`--no-demangle` is recommended: names then match `llvm-nm` exactly).
"""
import bisect
import collections
import ctypes
import json
import os
import re
import subprocess
import sys

CQLITE_COMMIT = "2e5b7a4cf6abe6f51b7677ecb7199e2cab087c0e"
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


LINE_RE = re.compile(r"^\s*([0-9a-fA-F]+)\s+(.*?)\s+\(([^)]*)\)\s*$")
SYM_RE = re.compile(r"^(.*)\+0x([0-9a-fA-F]+)$")


def parse_script(path):
    """Yield (ip, sym, symoff, dso) for each sample line."""
    with open(path, errors="replace") as fh:
        for line in fh:
            m = LINE_RE.match(line)
            if not m:
                continue
            ip, symoff, dso = int(m.group(1), 16), m.group(2), m.group(3)
            if ip >= 0xFFFF800000000000:  # kernel address leaked into a :u profile
                continue
            sym, off = symoff, 0
            sm = SYM_RE.match(symoff)
            if sm:
                sym, off = sm.group(1), int(sm.group(2), 16)
            yield ip, sym, off, dso


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
    """addr -> (innermost function, source file, outermost symbol) via addr2line -i -f -C."""
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


def main(argv):
    bench, script, binary = argv[1:4]
    out_path = argv[4] if len(argv) > 4 else None
    samples = list(parse_script(script))
    bin_real = os.path.realpath(binary)
    is_bench = lambda d: os.path.realpath(d) == bin_real if d.startswith("/") else False

    nm = nm_table(binary)
    libs = {}
    bases = {}
    for ip, sym, off, dso in samples:
        if dso.endswith("libc.so.6") and "libc" not in libs:
            libs["libc"] = StrippedLib(dso)
            bases["libc"] = libs["libc"].runtime_base(samples, os.path.splitext(script)[0] + ".mmap")
    # file address of each bench sample = symbol address + symoff
    keyed = {}
    unresolved = 0
    for ip, sym, off, dso in samples:
        if is_bench(dso):
            addrs = nm.get(sym)
            if addrs and len(set(addrs)) == 1:
                keyed[(sym, off)] = addrs[0] + off
            else:
                unresolved += 1
    chains = resolve_inline(binary, set(keyed.values()))

    total = collections.Counter()
    func_stats = {}
    n = 0
    for ip, sym, off, dso in samples:
        n += 1
        if is_bench(dso):
            fa = keyed.get((sym, off))
            frames = chains.get(fa) if fa is not None else None
            if frames and frames[0][0] != "??":
                fn, src = frames[0][0], frames[0][1].rsplit(":", 1)[0]
            else:
                fn, src = strip_hash(sym), ""
            phys = strip_hash(sym)
            d = "bench"
        else:
            fn, src, phys, d = strip_hash(sym), "", strip_hash(sym), dso
            if sym == "[unknown]" and dso.endswith("libc.so.6") and bases.get("libc") is not None:
                fn = phys = libs["libc"].name(ip - bases["libc"])
        # perf can show a bare address when no symbol is known
        b = classify(fn, binary if d == "bench" else d)
        total[b] += 1
        key = (fn, b)
        st = func_stats.setdefault(key, {"count": 0, "src": src, "phys": collections.Counter()})
        st["count"] += 1
        st["phys"][phys] += 1

    buckets = {b: total[b] / n for b in BUCKETS}
    ranked = sorted(func_stats.items(), key=lambda kv: -kv[1]["count"])
    top = ranked[:40]
    entry = lambda fn, b, st: {"function": fn, "source_file": st["src"], "bucket": b,
                               "share": st["count"] / n,
                               "physical_symbols": [x for x, _ in st["phys"].most_common(5)]}
    result = {
        "bench": bench,
        "cqlite_commit": CQLITE_COMMIT,
        "samples": n,
        "buckets": buckets,
        "top_functions": [entry(fn, b, st) for (fn, b), st in top],
        # additive to the documented shape: the report lists the top 15 own_decoding functions
        "own_decoding_top": [entry(fn, b, st) for (fn, b), st in ranked if b == "own_decoding"][:15],
    }
    if unresolved:
        print("ceiling: %d bench samples had no unique nm symbol (kept as physical symbol)" % unresolved,
              file=sys.stderr)
    text = json.dumps(result, indent=2) + "\n"
    if out_path:
        with open(out_path, "w") as fh:
            fh.write(text)
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
