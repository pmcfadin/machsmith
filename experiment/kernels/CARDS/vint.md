# Kernel card: `vint`

## 1. What it does

Decodes one unsigned variable-length integer (Cassandra's `writeUnsignedVInt` format) from the start of a byte slice. The count of leading one bits in the first byte is the number of extra bytes; the value is the remaining bits of the first byte followed by the extra bytes, big-endian. So `0xxxxxxx` is one byte, `10xxxxxx xxxxxxxx` is two bytes, and `0xFF` followed by eight bytes is nine bytes holding a full 64-bit value. The function consumes exactly `leading_ones + 1` bytes and ignores anything after them.

## 2. C signature

```rust
#[no_mangle] pub unsafe extern "C" fn msx_vint(ptr: *const u8, len: usize, out: *mut u64) -> usize;
```

## 3. Contract

- Success: returns the number of bytes consumed (1 to 9) and writes the decoded value to `*out`.
- Error (the input is empty, or shorter than the lead byte declares): returns 0.
- Nothing is written to `*out` on error.
- Bytes beyond those consumed are not examined and do not affect the result.
- Encodings are not checked for minimality: `[0x80, 0x00]` decodes to 0 and consumes 2 bytes.

Examples: `[0x00]` gives 1, value 0. `[0x80, 0x80]` gives 2, value 128. `[0xBF, 0xFF]` gives 2, value 0x3FFF. `[0xFE, 1, 2, 3, 4, 5, 6, 7]` gives 8, value 0x0001_0203_0405_0607. `[0xFF, 1, 2, 3, 4, 5, 6, 7, 8]` gives 9, value 0x0102_0304_0506_0708. `[0x80]` gives 0.

## 4. What you may assume

Nothing beyond the signature.

## 5. What you may NOT assume

- There are no readable bytes before `ptr` or at or after `ptr + len`. Reading them may fault.
- The length may be 0.
- The pointer has no alignment beyond that of `u8`.
- Bytes may have any value.
- `out` is valid for one `u64` write, and only `*out` may be written.

## 6. How CQLite calls it

CQLite calls `decode_unsigned` on a slice that starts at the current position and then advances by the returned byte count. It is used throughout SSTable parsing for lengths and offsets. The measurement replays it as a chain: start at offset 0, call, stop on error, fold the decoded value into a checksum, advance by the bytes consumed, repeat. The checksum is `acc = acc.rotate_left(5) ^ word` starting from 0.

## 7. Input statistics

(Filled in by Task 4.)

## 8. Source

Copied verbatim from CQLite. The only edits permitted inside a verbatim block are changing a visibility keyword and removing `debug!`/`trace!` calls. In this kernel: no logging macro calls were present or removed; the `///` doc comments above each item were not copied; visibility is `pub` in both places.

From `cqlite-core/src/parser/vint.rs` (`VIntError`, commit `2e5b7a4c`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VIntError {
    /// Input ended before the full VInt could be read (covers empty input).
    Truncated,
}
```

From `cqlite-core/src/parser/vint.rs` (`decode_unsigned`, commit `2e5b7a4c`):

```rust
#[inline]
pub fn decode_unsigned(input: &[u8]) -> Result<(u64, usize), VIntError> {
    let first = *input.first().ok_or(VIntError::Truncated)?;
    // A u8 has at most 8 leading ones, so `extra ∈ 0..=8` and `total ∈ 1..=9`
    // (== MAX_VINT_SIZE). No width can exceed the 9-byte VInt maximum.
    let extra = first.leading_ones() as usize;
    let total = extra + 1;
    if input.len() < total {
        return Err(VIntError::Truncated);
    }
    if extra == 0 {
        // Single byte: 0xxxxxxx
        return Ok((first as u64, 1));
    }
    // Load the `extra` continuation bytes as the low bits of a u64 via ONE
    // big-endian read. `rest[..extra]` is in-bounds: `input.len() >= total`.
    let (_, rest) = input.split_at(1);
    let mut be = [0u8; 8];
    be[8 - extra..].copy_from_slice(&rest[..extra]);
    let tail = u64::from_be_bytes(be);
    let value = if extra == 8 {
        // 0xFF lead: no data bits in the first byte; all 8 continuation bytes.
        tail
    } else {
        // Data bits carried in the first byte occupy the low `7 - extra` bits
        // (extra ∈ 1..=7 ⇒ data_bits_first ∈ 0..=6, so `1 << data_bits_first`
        // never overflows; extra == 7 ⇒ mask == 0, i.e. the 0xFE no-data case).
        let data_bits_first = 7 - extra;
        let mask = (1u64 << data_bits_first) - 1;
        // extra*8 ≤ 56 and (first & mask) ≤ 6 bits ⇒ ≤ 62 significant bits: no
        // shift overflow.
        ((first as u64 & mask) << (extra * 8)) | tail
    };
    Ok((value, total))
}
```

Type substitution: none. The shim:

```rust
pub unsafe extern "C" fn msx_vint(ptr: *const u8, len: usize, out: *mut u64) -> usize {
    let input = core::slice::from_raw_parts(ptr, len);
    match decode_unsigned(input) {
        Ok((value, consumed)) => { *out = value; consumed }
        Err(VIntError::Truncated) => 0,
    }
}
```

## Compiler output (12 November 2026, rustc 1.97.1, release, `lto = "fat"`, `panic = "abort"`)

- Code size of `msx_vint`: 186 bytes.
- Outgoing calls: only those the admissibility contract allows. No panic call is present.

## 9. This CPU

AMD Ryzen AI MAX+ 395 (Zen 5). Relevant feature flags from `/proc/cpuinfo`: BMI1, BMI2, LZCNT (reported as `abm`), POPCNT, MOVBE, AVX2, AVX-512 F, BW, VL, DQ, VBMI, VBMI2. Any instruction this CPU supports is allowed.
