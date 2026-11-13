import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import attribute  # noqa: E402

BENCH = "/w/target/cqlite-perfprof/perfprof/deps/read-abc123"

CASES = [
    ("cqlite_core::parser::vint::decode_unsigned", BENCH, "own_decoding"),
    ("cqlite_core::storage::sstable::reader::parsing::row_decoder::row_framing::frame", BENCH, "own_decoding"),
    ("cqlite_core::storage::sstable::index_reader::parse::parse_big_index_entry", BENCH, "own_decoding"),
    ("cqlite_core::storage::sstable::bti::node::BtiNode::find_child", BENCH, "own_decoding"),
    ("cqlite_core::storage::sstable::summary_reader::read", BENCH, "own_decoding"),
    ("cqlite_core::storage::sstable::row_cell_state_machine::step", BENCH, "own_decoding"),
    ("cqlite_core::storage::sstable::chunk_decompressor::decompress", BENCH, "other_cqlite"),
    ("cqlite_core::storage::sstable::reader::open", BENCH, "other_cqlite"),
    ("lz4_flex::block::decompress::decompress_internal", BENCH, "decompression"),
    ("ZSTD_decompressBlock_internal", BENCH, "decompression"),
    ("crc32fast::baseline::update_fast_16", BENCH, "checksum"),
    ("malloc", "/usr/lib/x86_64-linux-gnu/libc.so.6", "allocator"),
    ("__rust_alloc", BENCH, "allocator"),
    ("__memmove_avx512_unaligned_erms", "/usr/lib/x86_64-linux-gnu/libc.so.6", "memcpy"),
    ("crossbeam_channel::flavors::list::Channel<T>::send", BENCH, "sync"),
    ("std::sync::mpmc::list::Channel<T>::recv", BENCH, "sync"),
    ("criterion::routine::Routine::profile", BENCH, "bench_harness"),
    ("rustc_demangle::v0::Printer::print_path", BENCH, "bench_harness"),
    ("pprof::profiler::perf_signal_handler", BENCH, "bench_harness"),
    ("_Unwind_Find_FDE", "/usr/lib/x86_64-linux-gnu/libgcc_s.so.1", "bench_harness"),
    ("anything_at_all", "/usr/lib/x86_64-linux-gnu/libgcc_s.so.1", "bench_harness"),
    ("gimli::read::unit::Unit::new", BENCH, "bench_harness"),
    ("core::fmt::write", BENCH, "other"),
]


class ClassifyTest(unittest.TestCase):
    def test_table(self):
        for function, dso, want in CASES:
            with self.subTest(function=function, dso=dso):
                self.assertEqual(attribute.classify(function, dso), want)


def counts(n, **by_bucket):
    c = {b: 0 for b in attribute.BUCKETS}
    rest = n - sum(by_bucket.values())
    c.update(by_bucket)
    c["other"] += rest
    return c


class CombineTest(unittest.TestCase):
    def test_per_case_shares_sum_to_100(self):
        per = {"a": (100, counts(100, allocator=10, own_decoding=33)),
               "b": (300, counts(300, allocator=150, memcpy=7))}
        out = attribute.combine(per)
        for case, shares in out["per_case"].items():
            self.assertAlmostEqual(sum(shares.values()), 100.0, delta=0.1, msg=case)
        self.assertAlmostEqual(sum(out["workload"].values()), 100.0, delta=0.1)
        self.assertEqual(out["samples"], 400)

    def test_workload_weighted_by_samples(self):
        per = {"a": (100, counts(100, allocator=10)), "b": (300, counts(300, allocator=150))}
        out = attribute.combine(per)
        self.assertAlmostEqual(out["per_case"]["a"]["allocator"], 10.0)
        self.assertAlmostEqual(out["per_case"]["b"]["allocator"], 50.0)
        self.assertAlmostEqual(out["workload"]["allocator"], 40.0)


BIN = "/w/bench-abc"
LIBC = "/usr/lib/x86_64-linux-gnu/libc.so.6"


def frame(name, dso=BIN):
    return (name, dso)


class InclusiveTest(unittest.TestCase):
    def test_inclusive_counts_once_per_sample(self):
        chains = [
            [frame("_int_malloc", LIBC), frame("malloc", LIBC), frame("__rust_alloc"),
             frame("malloc", LIBC), frame("cqlite_core::a::b")],
            [frame("cqlite_core::a::b")],
        ]
        inc = attribute.inclusive_shares(chains)
        self.assertAlmostEqual(inc["allocator"], 50.0)
        self.assertAlmostEqual(inc["cqlite:cqlite_core::a::b"], 100.0)

    def test_named_frames(self):
        chains = [
            [frame("core::hash::sip::Hasher<S>::write"), frame("cqlite_core::types::CqlType::parse_with_depth")],
            [frame("__memcmp_evex_movbe", LIBC), frame("bcmp", LIBC)],
            [frame("core::ptr::drop_in_place<alloc::sync::Arc<str>>"),
             frame("<alloc::sync::Arc<str> as core::clone::Clone>::clone")],
            [frame("cqlite_core::value::Value::into_owned")],
            [frame("core::fmt::write")],
        ]
        inc = attribute.inclusive_shares(chains)
        self.assertAlmostEqual(inc["core::hash::sip"], 20.0)
        self.assertAlmostEqual(inc["CqlType::parse_with_depth"], 20.0)
        self.assertAlmostEqual(inc["bcmp/memcmp"], 20.0)
        self.assertAlmostEqual(inc["Arc<str> clone/drop"], 20.0)
        self.assertAlmostEqual(inc["Value::into_owned"], 20.0)
        self.assertEqual(inc["allocator"], 0.0)

    def test_harness_sample_counts_in_denominator_only(self):
        chains = [
            [frame("malloc", LIBC), frame("pprof::collector::Collector::add"), frame("cqlite_core::a::b")],
            [frame("malloc", LIBC), frame("cqlite_core::a::b")],
        ]
        n, hits, st = attribute.inclusive_counts(chains)
        self.assertEqual(n, 2)
        self.assertEqual(hits["allocator"], 1)
        self.assertEqual(hits["cqlite:cqlite_core::a::b"], 1)
        self.assertAlmostEqual(attribute.chain_stats_report(n, st)["inclusive_harness_samples"], 50.0)

    def test_cqlite_backtrace_function_is_not_harness(self):
        chains = [[frame("malloc", LIBC), frame("cqlite_core::error::backtrace_capture")]]
        self.assertEqual(attribute.inclusive_shares(chains)["allocator"], 100.0)

    def test_chain_reaches_root_and_depth(self):
        chains = [
            [frame("a"), frame("criterion::routine::f"), frame("main")],
            [frame("a"), frame("b")],
            [frame("a"), frame("b"), frame("c"), frame("_start")],
            [frame("x")],
        ]
        n, _, st = attribute.inclusive_counts(chains)
        rep = attribute.chain_stats_report(n, st)
        self.assertAlmostEqual(rep["chain_reaches_root"], 50.0)
        self.assertAlmostEqual(rep["median_chain_depth"], 2.5)
        self.assertEqual(rep["inclusive_harness_samples"], 0.0)

    def test_explicit_depths_override_chain_length(self):
        n, _, st = attribute.inclusive_counts([[frame("a"), frame("a")]], depths=[1])
        self.assertEqual(attribute.chain_stats_report(n, st)["median_chain_depth"], 1)

    def test_top_cqlite_limited_to_30(self):
        chains = [[frame("cqlite_core::f%d" % i)] for i in range(50)]
        inc = attribute.inclusive_shares(chains)
        self.assertEqual(len([k for k in inc if k.startswith("cqlite:")]), 30)


class ParseTest(unittest.TestCase):
    def test_callchain_blocks(self):
        import tempfile
        text = ("\t    55aa10 foo+0x10 (/w/bench)\n\t    55aa20 bar+0x4 (/w/bench)\n\n"
                "\t    7f0001 [unknown] (/lib/libc.so.6)\n\n")
        with tempfile.NamedTemporaryFile("w", suffix=".script", delete=False) as fh:
            fh.write(text)
        blocks = list(attribute.parse_blocks(fh.name))
        os.unlink(fh.name)
        self.assertEqual([len(b) for b in blocks], [2, 1])
        self.assertEqual(blocks[0][0], (0x55aa10, "foo", 0x10, "/w/bench"))

    def test_inlined_lines_are_dropped(self):
        import tempfile
        text = ("\t    55aa10 inl+0x10 (inlined)\n\t    55aa10 foo+0x10 (/w/bench)\n\n"
                "\t    55aa30 baz+0x1 (/w/bench)\n\n")
        with tempfile.NamedTemporaryFile("w", suffix=".script", delete=False) as fh:
            fh.write(text)
        blocks = list(attribute.parse_blocks(fh.name))
        os.unlink(fh.name)
        self.assertEqual([[f[1] for f in b] for b in blocks], [["foo"], ["baz"]])

    def test_kernel_leaf_sample_dropped(self):
        import tempfile
        text = ("\tffffffffb0400b90 [unknown] ([unknown])\n\t    55aa10 foo+0x10 (/w/bench)\n\n"
                "\t    55aa30 baz+0x1 (/w/bench)\n\n")
        with tempfile.NamedTemporaryFile("w", suffix=".script", delete=False) as fh:
            fh.write(text)
        blocks = list(attribute.parse_blocks(fh.name))
        os.unlink(fh.name)
        self.assertEqual(len(blocks), 1)

    def test_flat_lines_are_one_sample_each(self):
        import tempfile
        text = "\t    55aa10 foo+0x10 (/w/bench)\n\t    55aa20 bar+0x4 (/w/bench)\n"
        with tempfile.NamedTemporaryFile("w", suffix=".script", delete=False) as fh:
            fh.write(text)
        blocks = list(attribute.parse_blocks(fh.name, callgraph=False))
        os.unlink(fh.name)
        self.assertEqual([len(b) for b in blocks], [1, 1])


if __name__ == "__main__":
    unittest.main()
