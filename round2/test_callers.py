import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import callers  # noqa: E402

BIN = "/w/bench-abc"
LIBC = "/usr/lib/x86_64-linux-gnu/libc.so.6"


class StubContext:
    """0x1000-0x1fff is the bench binary (file address = runtime address - 0x1000),
    0x9000-0x9fff is libc. `frames` maps a bench file address to its inline chain."""

    def __init__(self, frames, plt=()):
        self.frames = frames
        self.plt = set(plt)

    def dso_of(self, addr):
        if 0x1000 <= addr < 0x2000:
            return BIN
        if 0x9000 <= addr < 0xa000:
            return LIBC
        return None

    def is_bench(self, dso):
        return dso == BIN

    def file_addr(self, addr):
        return addr - 0x1000

    def in_plt(self, file_addr):
        return file_addr in self.plt

    def inline_frames(self, file_addr):
        return self.frames.get(file_addr, [])


class ParseBrstackTest(unittest.TestCase):
    def test_parse_brstack_order(self):
        self.assertEqual(callers.parse_brstack("0x10/0x20/P/-/-/0 0x30/0x40/P/-/-/0"),
                         [(0x10, 0x20), (0x30, 0x40)])

    def test_parse_brstack_annotated(self):
        field = ("0x556abcdc5923 (/w/bench-abc)/0x7a313e35d750 (/usr/lib/libc.so.6)/P/-/-/0//NON_SPEC_CORRECT_PATH  "
                 "0x556abcebce8b (/w/bench-abc)/0x556abcdc58f0 (/w/bench-abc)/P/-/-/0//NON_SPEC_CORRECT_PATH")
        self.assertEqual(callers.parse_brstack(field),
                         [(0x556abcdc5923, 0x7a313e35d750), (0x556abcebce8b, 0x556abcdc58f0)])

    def test_parse_brstack_empty(self):
        self.assertEqual(callers.parse_brstack(""), [])
        self.assertEqual(callers.parse_brstack("   "), [])

    def test_parse_sample_line(self):
        line = ("     7a313e35d803 malloc+0xb3 (/usr/lib/x86_64-linux-gnu/libc.so.6) "
                "0x1010 (/w/bench-abc)/0x9100 (/usr/lib/x86_64-linux-gnu/libc.so.6)"
                "/P/-/-/0//NON_SPEC_CORRECT_PATH\n")
        frame, br = callers.parse_sample(line)
        self.assertEqual(frame, (0x7a313e35d803, "malloc", 0xb3, LIBC))
        self.assertEqual(br, [(0x1010, 0x9100)])
        frame, br = callers.parse_sample("     55aa10 foo+0x10 (/w/bench-abc)\n")
        self.assertEqual(frame, (0x55aa10, "foo", 0x10, BIN))
        self.assertEqual(br, [])


class CallerSiteTest(unittest.TestCase):
    def test_skips_libc_internal_calls(self):
        ctx = StubContext({0x10: [("cqlite_core::a::b", "/s/cqlite-core/src/a.rs:10")]})
        site = callers.caller_site(ctx, LIBC, [(0x9010, 0x9100), (0x1010, 0x8000)])
        self.assertEqual(site, callers.format_site("cqlite_core::a::b", "/s/cqlite-core/src/a.rs:10"))

    def test_none_when_no_bench_entry(self):
        ctx = StubContext({})
        self.assertIsNone(callers.caller_site(ctx, LIBC, [(0x9010, 0x9100), (0x5000, 0x9000)]))

    def test_skips_plt(self):
        ctx = StubContext({0x20: [("cqlite_core::c", "/s/cqlite-core/src/c.rs:3")]}, plt=[0x10])
        site = callers.caller_site(ctx, LIBC, [(0x1010, 0x9000), (0x1020, 0x1010)])
        self.assertEqual(site, callers.format_site("cqlite_core::c", "/s/cqlite-core/src/c.rs:3"))

    def test_prefers_cqlite_frame_in_inline_chain(self):
        ctx = StubContext({0x10: [("alloc::raw_vec::RawVec<T,A>::grow_one",
                                   "/rustc/x/library/alloc/src/raw_vec.rs:5"),
                                  ("cqlite_core::x::y", "/s/cqlite-core/src/x.rs:42"),
                                  ("cqlite_core::x::outer", "/s/cqlite-core/src/x.rs:99")]})
        site = callers.caller_site(ctx, LIBC, [(0x1010, 0x9000)])
        self.assertEqual(site, callers.format_site("cqlite_core::x::y", "/s/cqlite-core/src/x.rs:42"))
        self.assertTrue(site.startswith("cqlite_core::x::y "))

    def test_innermost_frame_without_cqlite(self):
        ctx = StubContext({0x10: [("core::fmt::write", "/rustc/x/library/core/src/fmt/mod.rs:7"),
                                  ("criterion::bencher::Bencher::iter", "/c/bencher.rs:1")]})
        site = callers.caller_site(ctx, LIBC, [(0x1010, 0x9000)])
        self.assertEqual(site, callers.format_site("core::fmt::write", "/rustc/x/library/core/src/fmt/mod.rs:7"))


    def test_skips_allocator_glue_entries(self):
        # __rdl_alloc -> malloc is the most recent bench call; its chain is allocator glue only, so the
        # caller is the older entry that called into the shim.
        ctx = StubContext({0x10: [("<std::alloc::System as core::alloc::global::GlobalAlloc>::alloc",
                                   "/rustc/x/library/std/src/sys/alloc/unix.rs:14"),
                                  ("__rustc::__rdl_alloc", "/rustc/x/library/std/src/alloc.rs:1")],
                           0x30: [("alloc::raw_vec::finish_grow", "/rustc/x/library/alloc/src/raw_vec.rs:9")],
                           0x50: [("<alloc::vec::Vec<T>>::push", "/rustc/x/library/alloc/src/vec/mod.rs:3"),
                                  ("cqlite_core::p::q", "/s/cqlite-core/src/p.rs:8")]})
        site = callers.caller_site(ctx, LIBC, [(0x9010, 0x9100), (0x1010, 0x9000), (0x1030, 0x1010),
                                               (0x1050, 0x1030)])
        self.assertEqual(site, callers.format_site("cqlite_core::p::q", "/s/cqlite-core/src/p.rs:8"))

    def test_fallback_skips_glue_frames(self):
        ctx = StubContext({0x10: [("<std::alloc::System as core::alloc::global::GlobalAlloc>::alloc",
                                   "/r/unix.rs:14"),
                                  ("alloc::alloc::alloc", "/r/alloc.rs:2"),
                                  ("<str>::to_lowercase", "/rustc/x/library/alloc/src/str.rs:400"),
                                  ("core::fmt::write", "/rustc/x/library/core/src/fmt/mod.rs:7")]})
        site = callers.caller_site(ctx, LIBC, [(0x1010, 0x9000)])
        self.assertEqual(site, callers.format_site("<str>::to_lowercase", "/rustc/x/library/alloc/src/str.rs:400"))

    def test_only_glue_entries_is_no_caller(self):
        ctx = StubContext({0x10: [("__rdl_alloc", "/rustc/x/library/std/src/alloc.rs:1")]})
        self.assertIsNone(callers.caller_site(ctx, LIBC, [(0x1010, 0x9000)]))


class ConsistencyTest(unittest.TestCase):
    SITE = [("cqlite_core::s::t", "/s/cqlite-core/src/s.rs:5")]

    def test_newer_call_into_ordinary_bench_code_is_inconsistent(self):
        # newest entry: ordinary bench code calling ordinary bench code (skid, or a finished allocation)
        ctx = StubContext({0x10: [("core::hash::sip::Hasher::write", "/r/sip.rs:1")],
                           0x20: [("<core::hash::sip::Hasher as core::hash::Hasher>::write_u8", "/r/mod.rs:363")],
                           0x40: self.SITE})
        stack = [(0x1010, 0x1020), (0x1040, 0x9000)]
        self.assertEqual(callers.classify_caller(ctx, LIBC, stack), (callers.INCONSISTENT, None))
        self.assertIsNone(callers.caller_site(ctx, LIBC, stack))

    def test_credited_call_must_reach_libc_or_glue(self):
        ctx = StubContext({0x40: self.SITE, 0x60: [("cqlite_core::u::v", "/s/cqlite-core/src/u.rs:1")]})
        self.assertEqual(callers.classify_caller(ctx, LIBC, [(0x1040, 0x1060)]), (callers.INCONSISTENT, None))

    def test_consistent_chain_through_glue_is_credited(self):
        ctx = StubContext({0x10: [("__rustc::__rdl_alloc", "/r/alloc.rs:1")], 0x40: self.SITE})
        status, site = callers.classify_caller(ctx, LIBC, [(0x9020, 0x9030), (0x1010, 0x9000), (0x1040, 0x1010)])
        self.assertEqual(status, callers.CREDITED)
        self.assertEqual(site, callers.format_site(*self.SITE[0]))

    def test_unresolved_bench_site_stops_walk_as_no_caller(self):
        ctx = StubContext({0x10: [("??", "??:0")], 0x40: self.SITE})
        self.assertEqual(callers.classify_caller(ctx, LIBC, [(0x1010, 0x9000), (0x1040, 0x9000)]),
                         (callers.NO_CALLER, None))
        ctx = StubContext({0x40: self.SITE})  # no frames at all
        self.assertEqual(callers.classify_caller(ctx, LIBC, [(0x1010, 0x9000), (0x1040, 0x9000)]),
                         (callers.NO_CALLER, None))


class GlueTest(unittest.TestCase):
    def test_glue_names(self):
        glue = ["core::ptr::drop_glue::<alloc::raw_vec::RawVec<u8>>",
                "core::ptr::drop_in_place::<alloc::vec::Vec<cqlite_core::schema::Column>>",
                "core::ptr::drop_glue::<alloc::string::String>",
                "core::ptr::drop_glue::<alloc::boxed::Box<[u8]>>",
                "<alloc::vec::Vec<u8>>::with_capacity_in",
                "<alloc::vec::Vec<u8>>::with_capacity",
                "<alloc::vec::Vec<cqlite_core::x::ColumnToParse>>::reserve",
                "<alloc::vec::Vec<u8>>::reserve_exact",
                "<alloc::vec::Vec<u8>>::push",
                "<alloc::vec::Vec<u8>>::grow_one",
                "hashbrown::alloc::inner::do_alloc::<alloc::alloc::Global>",
                "__rustc::__rdl_alloc",
                "<std::alloc::System as core::alloc::global::GlobalAlloc>::alloc"]
        for name in glue:
            with self.subTest(name=name):
                self.assertTrue(callers.is_glue(name))
        for name in ["core::ptr::drop_glue::<cqlite_core::schema::Column>",
                     "<cqlite_core::schema::Column as core::clone::Clone>::clone",
                     "<alloc::vec::Vec<u8> as core::clone::Clone>::clone",
                     "core::hash::sip::Hasher::write", "<str>::to_lowercase"]:
            with self.subTest(name=name):
                self.assertFalse(callers.is_glue(name))


class SummarizeTest(unittest.TestCase):
    def test_empty_brstack_counts_as_no_branch_data(self):
        # (case, bucket, has_brstack, site)
        records = [
            ("c", "allocator", False, None),
            ("c", "allocator", True, None),
            ("c", "allocator", True, "cqlite_core::a (a.rs:1)"),
            ("c", "other", True, None),
            ("c", "allocator", True, callers.INCONSISTENT_SITE),
        ]
        out = callers.summarize(records)
        self.assertEqual(out["samples"], 5)
        self.assertEqual(out["bucket_samples"], {"allocator": 4, "memcpy": 0})
        self.assertAlmostEqual(out["brstack_empty_pct"], 20.0)
        # denominators: the three allocator samples that have branch data
        self.assertAlmostEqual(out["no_caller_pct"]["allocator"], 100.0 / 3)
        self.assertAlmostEqual(out["inconsistent_pct"]["allocator"], 100.0 / 3)
        self.assertEqual(out["no_caller_pct"]["memcpy"], 0.0)
        self.assertEqual(out["inconsistent_pct"]["memcpy"], 0.0)
        top = out["sites"]["allocator"][0]
        self.assertEqual(top["site"], "cqlite_core::a (a.rs:1)")
        self.assertAlmostEqual(top["share_of_bucket"], 100.0 / 3)
        self.assertAlmostEqual(top["share_of_workload"], 20.0)
        self.assertEqual(out["other_sites_pct"]["allocator"], 0.0)
        case_top = out["per_case_sites"]["c"]["allocator"][0]
        self.assertEqual(case_top["site"], "cqlite_core::a (a.rs:1)")
        self.assertAlmostEqual(case_top["share_of_case"], 20.0)
        self.assertNotIn("share_of_workload", case_top)

    def test_bucket_parts_sum_to_100(self):
        records = [("c", "memcpy", True, "site%02d" % i) for i in range(30)]
        records += [("c", "memcpy", True, None), ("c", "memcpy", True, callers.INCONSISTENT_SITE)]
        out = callers.summarize(records)
        self.assertEqual(len(out["sites"]["memcpy"]), 25)
        total = (sum(e["share_of_bucket"] for e in out["sites"]["memcpy"]) + out["other_sites_pct"]["memcpy"]
                 + out["no_caller_pct"]["memcpy"] + out["inconsistent_pct"]["memcpy"])
        self.assertAlmostEqual(total, 100.0)
        self.assertAlmostEqual(out["other_sites_pct"]["memcpy"], 500.0 / 32)

    def test_only_empty_samples_is_not_no_caller(self):
        out = callers.summarize([("c", "memcpy", False, None), ("c", "memcpy", False, None)])
        self.assertAlmostEqual(out["brstack_empty_pct"], 100.0)
        self.assertEqual(out["no_caller_pct"]["memcpy"], 0.0)


if __name__ == "__main__":
    unittest.main()
