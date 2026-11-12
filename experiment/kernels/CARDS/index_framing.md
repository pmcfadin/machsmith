# Kernel card: `index_framing`

## 1. What it does

Frames one entry of a BIG-format Cassandra `Index.db` file. An entry starts with a big-endian `u16` key length, then that many key bytes, then two unsigned VInts (the partition's data-file offset and the length of the promoted index that follows), then the promoted payload. The function returns how many bytes the framing itself occupies (key length field, key, both VInts) and the declared promoted payload length. It does not check the payload against the input. The VInt format is the one described in the `vint` card.

## 2. C signature

```rust
#[repr(C)] pub struct FramingOut { pub framing_len: u64, pub promoted_len: u64 }
#[no_mangle] pub unsafe extern "C" fn msx_index_framing(ptr: *const u8, len: usize, out: *mut FramingOut) -> u32;
```

## 3. Contract

- Success: returns 0 and writes `framing_len` (bytes consumed by the framing) and `promoted_len` (the declared payload length) to `*out`.
- Failure: the CQLite function is a nom parser. Every failure in this input space is a `nom::Err::Error` with `ErrorKind::Eof` (input too short for the length field, the key, or either VInt). The return value is `1 + (ErrorKind::Eof as u32)`. If the function ever produced `nom::Err::Failure` the status would be computed the same way, and `nom::Err::Incomplete` would return `u32::MAX`; neither occurs.
- Nothing is written to `*out` on error.
- `promoted_len` is not validated against the input.

Examples: `[0x00, 0x02, b'a', b'b', 0x05, 0x00]` returns 0 with `framing_len = 6`, `promoted_len = 0` (a trailing byte changes nothing). `[0x00, 0x00, 0x05, 0x81, 0x00]` returns 0 with `framing_len = 5`, `promoted_len = 256`. Every proper prefix of the first example returns `1 + (ErrorKind::Eof as u32)`.

## 4. What you may assume

Nothing beyond the signature.

## 5. What you may NOT assume

- There are no readable bytes before `ptr` or at or after `ptr + len`. Reading them may fault.
- The length may be 0.
- The pointer has no alignment beyond that of `u8`.
- Bytes may have any value.
- `out` is valid for one `FramingOut` write, and only `*out` may be written.

## 6. How CQLite calls it

CQLite calls `parse_big_index_entry_framing` on the index bytes at the start of an entry to validate its framing and learn the entry's total length, `framing_len + promoted_len`, then moves to the next entry. The measurement replays an `Index.db` file as a chain: start at offset 0, call, stop on error, fold `framing_len` then `promoted_len` into the checksum, advance by `framing_len + promoted_len`, and stop if that passes the end of the segment. The entry that passes the end is still counted and folded. The checksum is `acc = acc.rotate_left(5) ^ word` starting from 0.

## 7. Input statistics

(Filled in by Task 4.)

## 8. Source

Copied verbatim from CQLite. The only edits permitted inside a verbatim block are changing a visibility keyword and removing `debug!`/`trace!` calls. In this kernel: no logging macro calls were present or removed; the `///` doc comments above each item were not copied; `pub(crate)` on `parse_big_index_entry_framing` became `pub`. `#[inline(always)]` was added to the copy of `parse_big_index_entry_framing` (CQLite has no inline attribute on it) so that it compiles into the exported function. The helpers `decode_unsigned` and `VIntError` are the ones shown in the `vint` card, and `parse_vuint` calls them. Dependency: `nom = "=7.1.3"`, supplying `be_u16` (`nom::number::complete`), `take` (`nom::bytes::complete`) and `IResult`.

From `cqlite-core/src/parser/vint.rs` (`parse_vuint`, commit `2e5b7a4c`):

```rust
pub fn parse_vuint(input: &[u8]) -> IResult<&[u8], u64> {
    // Issue #1638 (J4): thin nom adapter over the one canonical `decode_unsigned`.
    match decode_unsigned(input) {
        Ok((value, consumed)) => Ok((&input[consumed..], value)),
        Err(_) => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Eof,
        ))),
    }
}
```

From `cqlite-core/src/storage/sstable/index_reader/parse.rs` (`parse_big_index_entry_framing`, commit `2e5b7a4c`):

```rust
#[inline(always)] // added on the copy; CQLite's function has no inline attribute, and plain #[inline] left it out of line
pub fn parse_big_index_entry_framing(input: &[u8]) -> IResult<&[u8], (usize, u64)> {
    let original_len = input.len();
    let (input, key_len) = be_u16(input)?;
    let (input, _key_bytes) = take(key_len)(input)?;
    let (input, _data_offset) = parse_vuint(input)?;
    let (remaining, promoted_len) = parse_vuint(input)?;
    let framing_len = original_len - remaining.len();
    Ok((remaining, (framing_len, promoted_len)))
}
```

`decode_unsigned` and `VIntError` (copied verbatim, see the `vint` card):

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
pub unsafe extern "C" fn msx_index_framing(ptr: *const u8, len: usize, out: *mut FramingOut) -> u32 {
    let input = core::slice::from_raw_parts(ptr, len);
    match parse_big_index_entry_framing(input) {
        Ok((_, (framing_len, promoted_len))) => {
            *out = FramingOut { framing_len: framing_len as u64, promoted_len };
            0
        }
        Err(nom::Err::Error(e)) | Err(nom::Err::Failure(e)) => 1 + (e.code as u32),
        Err(nom::Err::Incomplete(_)) => u32::MAX,
    }
}
```

## Compiler output (12 November 2026, rustc 1.97.1, release, `lto = "fat"`, `panic = "abort"`)

- Code size of `msx_index_framing`: 368 bytes.
- Outgoing calls: besides the calls the admissibility contract allows, two calls to panic paths that the compiler could not prove dead: `core::panicking::panic_fmt` (from `split_at` inside nom's `take`) and `core::slice::index::slice_index_fail` (from slicing the input past the second VInt). Neither is reachable: `take` is entered only after `key_len <= remaining` was checked, and the slice is taken only after `input.len() >= leading_ones + 1` was checked. A seeded random test over 10,000 inputs of length 0 to 40 exercises the function with panics enabled and finds none.

## 9. This CPU

AMD Ryzen AI MAX+ 395 (Zen 5). Relevant feature flags from `/proc/cpuinfo`: BMI1, BMI2, LZCNT (reported as `abm`), POPCNT, MOVBE, AVX2, AVX-512 F, BW, VL, DQ, VBMI, VBMI2. Any instruction this CPU supports is allowed.
