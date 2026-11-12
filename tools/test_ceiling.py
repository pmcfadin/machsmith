import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import ceiling  # noqa: E402

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
                self.assertEqual(ceiling.classify(function, dso), want)


if __name__ == "__main__":
    unittest.main()
