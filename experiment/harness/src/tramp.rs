//! The ABI-checking trampoline.
//!
//! `call4` calls a kernel with hostile register state (distinct canaries in every
//! callee-saved register, garbage in the scratch and unused argument registers, a
//! clean vector state) and audits what the kernel left behind.

use crate::spec::RawFn;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum AbiViolation {
    CalleeSaved,
    StackPointer,
    DirectionFlag,
    Mxcsr,
    UpperVectorState,
}

#[repr(C)]
struct Raw {
    ret: u64,
    mask: u64,
}

extern "C" {
    // (f, a, b, c, d, xgetbv1_flag) -> (kernel rax, violation bitmask)
    fn msx_call4_raw(f: RawFn, a: u64, b: u64, c: u64, d: u64, flag: u64) -> Raw;
}

// Frame (after six pushes and `sub rsp, 40`, rsp is 16-byte aligned at the call):
//   [rsp+0]  MXCSR before      [rsp+4]  MXCSR after
//   [rsp+16] kernel address    [rsp+24] self-referential marker (holds its own address)
//   [rsp+32] flag: xgetbv(1) is usable
// r14 holds `rsp ^ K`, so the stack pointer can be recovered if the kernel moved it.
// Result bits: 1 callee-saved, 2 stack pointer, 4 direction flag, 8 MXCSR, 16 upper vector state.
core::arch::global_asm!(
    ".text",
    ".globl msx_call4_raw",
    ".type msx_call4_raw,@function",
    "msx_call4_raw:",
    "push rbp",
    "push rbx",
    "push r12",
    "push r13",
    "push r14",
    "push r15",
    "sub rsp, 40",
    "stmxcsr [rsp]",
    "mov [rsp+16], rdi",
    "mov [rsp+32], r9",
    "lea rax, [rsp+24]",
    "mov [rsp+24], rax",
    "mov rdi, rsi",
    "mov rsi, rdx",
    "mov rdx, rcx",
    "mov rcx, r8",
    "test qword ptr [rsp+32], 1",
    "jz .Lmsx_pre_done",
    "vzeroupper",
    ".Lmsx_pre_done:",
    "mov rbx, 0x1111111111111111",
    "mov rbp, 0x2222222222222222",
    "mov r12, 0x3333333333333333",
    "mov r13, 0x4444444444444444",
    "mov r15, 0x5555555555555555",
    "mov r14, rsp",
    "mov rax, 0x6B1D6B1D6B1D6B1D",
    "xor r14, rax",
    "mov rax, 0xA1A1A1A1A1A1A1A1",
    "mov r8,  0xB2B2B2B2B2B2B2B2",
    "mov r9,  0xC3C3C3C3C3C3C3C3",
    "mov r10, 0xD4D4D4D4D4D4D4D4",
    "mov r11, 0xE5E5E5E5E5E5E5E5",
    "call qword ptr [rsp+16]",
    // --- audit ---
    "pushfq",
    "pop r10",
    "cld",
    "mov r8, rax",
    "xor r9d, r9d",
    "lea rdx, [rsp+24]",
    "cmp qword ptr [rdx], rdx",
    "je .Lmsx_sp_ok",
    "or r9d, 2",
    "mov rdx, r14",
    "mov rcx, 0x6B1D6B1D6B1D6B1D",
    "xor rdx, rcx",
    "lea rcx, [rdx+24]",
    "cmp qword ptr [rcx], rcx",
    "jne .Lmsx_fatal",
    "mov rsp, rdx",
    ".Lmsx_sp_ok:",
    "mov rdx, 0x1111111111111111",
    "cmp rbx, rdx",
    "jne .Lmsx_cs",
    "mov rdx, 0x2222222222222222",
    "cmp rbp, rdx",
    "jne .Lmsx_cs",
    "mov rdx, 0x3333333333333333",
    "cmp r12, rdx",
    "jne .Lmsx_cs",
    "mov rdx, 0x4444444444444444",
    "cmp r13, rdx",
    "jne .Lmsx_cs",
    "mov rdx, 0x5555555555555555",
    "cmp r15, rdx",
    "jne .Lmsx_cs",
    "mov rdx, rsp",
    "mov rcx, 0x6B1D6B1D6B1D6B1D",
    "xor rdx, rcx",
    "cmp r14, rdx",
    "jne .Lmsx_cs",
    "jmp .Lmsx_cs_done",
    ".Lmsx_cs:",
    "or r9d, 1",
    ".Lmsx_cs_done:",
    "test r10d, 0x400",
    "jz .Lmsx_df_ok",
    "or r9d, 4",
    ".Lmsx_df_ok:",
    "stmxcsr [rsp+4]",
    "mov edx, [rsp+4]",
    "cmp edx, [rsp]",
    "je .Lmsx_mx_ok",
    "or r9d, 8",
    "ldmxcsr [rsp]",
    ".Lmsx_mx_ok:",
    "test qword ptr [rsp+32], 1",
    "jz .Lmsx_up_done",
    "mov ecx, 1",
    "xgetbv",
    "test eax, 0x44",
    "jz .Lmsx_up_clean",
    "or r9d, 16",
    ".Lmsx_up_clean:",
    "vzeroupper",
    ".Lmsx_up_done:",
    "mov rax, r8",
    "mov rdx, r9",
    "add rsp, 40",
    "pop r15",
    "pop r14",
    "pop r13",
    "pop r12",
    "pop rbx",
    "pop rbp",
    "ret",
    ".Lmsx_fatal:",
    "ud2",
    ".size msx_call4_raw, .-msx_call4_raw",
);

/// Whether `xgetbv` with `ecx = 1` (XINUSE) may be executed: AVX present and
/// `CPUID.(EAX=0DH,ECX=1):EAX` bit 2 set.
fn xgetbv1_usable() -> bool {
    static CELL: OnceLock<bool> = OnceLock::new();
    *CELL.get_or_init(|| {
        use core::arch::x86_64::{__cpuid, __cpuid_count};
        if !is_x86_feature_detected!("avx") {
            return false;
        }
        let max = __cpuid(0).eax;
        max >= 0xD && (__cpuid_count(0xD, 1).eax >> 2) & 1 == 1
    })
}

/// True when `call4` can detect dirty upper vector state on this CPU.
pub fn upper_state_check_available() -> bool {
    xgetbv1_usable()
}

/// Call `f(a, b, c, d)` under the System V ABI with hostile register state, and audit the state on return.
///
/// On success returns the full 64-bit `rax` (a kernel returning a 32-bit value
/// leaves the upper half undefined; callers mask).
///
/// # Safety
/// `f` must be a function of up to four integer arguments, and the arguments must
/// be valid for whatever it does with them.
pub unsafe fn call4(f: RawFn, a: u64, b: u64, c: u64, d: u64) -> Result<u64, AbiViolation> {
    let r = msx_call4_raw(f, a, b, c, d, xgetbv1_usable() as u64);
    let m = r.mask;
    if m & 1 != 0 {
        Err(AbiViolation::CalleeSaved)
    } else if m & 2 != 0 {
        Err(AbiViolation::StackPointer)
    } else if m & 4 != 0 {
        Err(AbiViolation::DirectionFlag)
    } else if m & 8 != 0 {
        Err(AbiViolation::Mxcsr)
    } else if m & 16 != 0 {
        Err(AbiViolation::UpperVectorState)
    } else {
        Ok(r.ret)
    }
}
