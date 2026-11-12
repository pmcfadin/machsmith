# Kernel card: `bti_sparse_find`

## 1. What it does

Looks up one byte in the transitions of a sparse trie node from CQLite's BTI (trie) index. The node holds up to 256 transitions, each a pair of a byte label and a child pointer (a 64-bit absolute offset), sorted ascending by byte with no duplicates. The function binary-searches for the transition whose byte equals the query and returns its child. The on-disk node is decoded into the in-memory array before this point; the function sees only the array.

## 2. C signature

```rust
#[repr(C)] pub struct Transition { pub byte: u8, pub child: u64 }      // size 16, align 8; padding bytes unspecified
#[no_mangle] pub unsafe extern "C" fn msx_bti_sparse_find(t: *const Transition, n: usize, byte: u32, out: *mut u64) -> u32;
```

## 3. Contract

- Found: returns 1 and writes the child of the transition whose `.byte == (byte as u8)` to `*out`.
- Not found, or `n == 0`: returns 0.
- Nothing is written to `*out` on error (not found).
- Only the low 8 bits of `byte` are meaningful.

Example with transitions `[(0x10, 100), (0x20, 200), (0x30, 300)]`: byte `0x20` returns 1 and writes 200. Bytes `0x0F`, `0x11`, `0x2F`, `0x31`, `0xFF` return 0. Byte `0xABCD_EF20` returns 1 and writes 200.

## 4. What you may assume

- The transitions are sorted ascending by `byte` with no duplicates.
- `n <= 256`.

## 5. What you may NOT assume

- There are no readable bytes before `t` or at or after `t + n` (that is, `t + n * 16` bytes). Reading them may fault.
- `n` may be 0.
- The pointer has no alignment beyond that of the type, which is 8.
- Bytes may have any value; padding bytes inside a `Transition` (bytes 1 to 7 of each element) are unspecified.
- The upper 24 bits of the `byte` argument are garbage.
- `out` is valid for one `u64` write, and only `*out` may be written.

## 6. How CQLite calls it

CQLite walks a trie from the root: at each sparse node it calls `find_child` with the next byte of the key and, if found, follows the child pointer. The measurement replays recorded nodes as a segment of records, each `[u32 LE n][u32 LE byte][n x 16 bytes]` where each 16-byte element is `[byte][7 zero bytes][u64 LE child]`: read a record, call the function with `t` pointing at its elements, fold the return value and, when found, the child into the checksum, advance to the next record. The walk stops if the remaining bytes cannot hold a header or the `n x 16` payload. The checksum is `acc = acc.rotate_left(5) ^ word` starting from 0. The segment base is 8-byte aligned.

## 7. Input statistics

(Filled in by Task 4.)

## 8. Source

Copied verbatim from CQLite. The only edits permitted inside a verbatim block are changing a visibility keyword and removing `debug!`/`trace!` calls; none were present or removed.

Type substitution: CQLite's `Transition` is `{ byte: u8, child: SizedPointer }` with `SizedPointer { distance: u64 }` and no `repr`. This kernel uses the `#[repr(C)]` `Transition { byte: u8, child: u64 }` from the signature above, which has the same 16-byte shape; the child is the plain `u64`. The search expression is unchanged, so `&transitions[idx].child` is a `&u64` here instead of a `&SizedPointer`.

From `cqlite-core/src/storage/sstable/bti/node.rs` (`BtiNode::find_child (Sparse arm)`, commit `2e5b7a4c`):

```rust
    transitions
        .binary_search_by_key(&byte, |t| t.byte)
        .ok()
        .map(|idx| &transitions[idx].child)
```

The shim:

```rust
pub unsafe extern "C" fn msx_bti_sparse_find(t: *const Transition, n: usize, byte: u32, out: *mut u64) -> u32 {
    let transitions = core::slice::from_raw_parts(t, n);
    match find_sparse(transitions, byte as u8) {
        Some(child) => { *out = *child; 1 }
        None => 0,
    }
}
```

where `find_sparse(transitions, byte)` is the verbatim expression above applied to `transitions` and `byte`.

## Compiler output (12 November 2026, rustc 1.97.1, release, `lto = "fat"`, `panic = "abort"`)

- Code size of `msx_bti_sparse_find`: 96 bytes.
- Outgoing calls: none.

## 9. This CPU

AMD Ryzen AI MAX+ 395 (Zen 5). Relevant feature flags from `/proc/cpuinfo`: BMI1, BMI2, LZCNT (reported as `abm`), POPCNT, MOVBE, AVX2, AVX-512 F, BW, VL, DQ, VBMI, VBMI2. Any instruction this CPU supports is allowed.
