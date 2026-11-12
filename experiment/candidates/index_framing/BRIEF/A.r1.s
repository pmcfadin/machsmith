
../work/build/A/r1/libmsx_kernels.so:	file format elf64-x86-64

Disassembly of section .text:

0000000000011bc0 <msx_index_framing>:
; pub unsafe extern "C" fn msx_index_framing(ptr: *const u8, len: usize, out: *mut FramingOut) -> u32 {
   11bc0:      	pushq	%r15
   11bc2:      	pushq	%r14
   11bc4:      	pushq	%r13
   11bc6:      	pushq	%r12
   11bc8:      	pushq	%rbx
   11bc9:      	subq	$0x10, %rsp
   11bcd:      	movl	$0x18, %eax
;   if input.input_len() < bound {
   11bd2:      	cmpq	$0x2, %rsi
   11bd6:      	jb	0x11cf3 <msx_index_framing+0x133>
;       res = (res << 8) + byte as u16;
   11bdc:      	movzwl	(%rdi), %ecx
   11bdf:      	rolw	$0x8, %cx
   11be3:      	movzwl	%cx, %ecx
   11be6:      	leaq	-0x2(%rsi), %rbx
   11bea:      	xorl	%r8d, %r8d
;     if self.len() >= count {
   11bed:      	cmpq	%rcx, %rbx
   11bf0:      	cmovbq	%rbx, %r8
;   move |i: Input| match i.slice_index(c) {
   11bf4:      	jb	0x11cf3 <msx_index_framing+0x133>
   11bfa:      	subq	%r8, %rcx
   11bfd:      	cmpq	%rcx, %rbx
   11c00:      	jb	0x11d01 <msx_index_framing+0x141>
   11c06:      	subq	%rcx, %rbx
   11c09:      	je	0x11cf3 <msx_index_framing+0x133>
   11c0f:      	addq	$0x2, %rdi
   11c13:      	addq	%rcx, %rdi
;     let first = *input.first().ok_or(VIntError::Truncated)?;
   11c16:      	movzbl	(%rdi), %ecx
   11c19:      	notb	%cl
   11c1b:      	movzbl	%cl, %r8d
   11c1f:      	movl	$0xf, %ecx
   11c24:      	bsrl	%r8d, %ecx
   11c28:      	xorl	$0x7, %ecx
;     if input.len() < total {
   11c2b:      	cmpq	%rcx, %rbx
   11c2e:      	jbe	0x11cf3 <msx_index_framing+0x133>
;     let total = extra + 1;
   11c34:      	leaq	0x1(%rcx), %r8
   11c38:      	subq	%r8, %rbx
   11c3b:      	je	0x11cf3 <msx_index_framing+0x133>
   11c41:      	addq	%rdi, %rcx
   11c44:      	incq	%rcx
;     let first = *input.first().ok_or(VIntError::Truncated)?;
   11c47:      	movzbl	(%rcx), %r15d
   11c4b:      	movl	%r15d, %edi
   11c4e:      	notb	%dil
   11c51:      	movzbl	%dil, %edi
   11c55:      	movl	$0xf, %r14d
   11c5b:      	bsrl	%edi, %r14d
   11c5f:      	xorl	$0x7, %r14d
;     if input.len() < total {
   11c63:      	cmpq	%r14, %rbx
   11c66:      	jbe	0x11cf3 <msx_index_framing+0x133>
;     if extra == 0 {
   11c6c:      	testq	%r14, %r14
   11c6f:      	je	0x11cd9 <msx_index_framing+0x119>
   11c71:      	movq	%rsi, %r12
   11c74:      	movq	%rdx, %r13
;     let mut be = [0u8; 8];
   11c77:      	movq	$0x0, 0x8(%rsp)
   11c80:      	incq	%rcx
   11c83:      	leaq	0x8(%rsp), %rdi
   11c88:      	subq	%r14, %rdi
   11c8b:      	addq	$0x8, %rdi
   11c8f:      	movq	%rcx, %rsi
   11c92:      	movq	%r14, %rdx
   11c95:      	callq	*0x3adb5(%rip)          # 0x4ca50 <writev+0x4ca50>
;     let tail = u64::from_be_bytes(be);
   11c9b:      	movq	0x8(%rsp), %rax
   11ca0:      	bswapq	%rax
;     let value = if extra == 8 {
   11ca3:      	cmpl	$0xff, %r15d
   11caa:      	je	0x11ccb <msx_index_framing+0x10b>
   11cac:      	movb	$0x7, %cl
;         let mask = (1u64 << data_bits_first) - 1;
   11cae:      	subb	%r14b, %cl
   11cb1:      	movl	$0xffffffff, %edx       # imm = 0xFFFFFFFF
   11cb6:      	shll	%cl, %edx
   11cb8:      	notl	%edx
;         ((first as u64 & mask) << (extra * 8)) | tail
   11cba:      	andl	%edx, %r15d
   11cbd:      	leal	(,%r14,8), %ecx
   11cc5:      	shlq	%cl, %r15
   11cc8:      	orq	%r15, %rax
   11ccb:      	movq	%r13, %rdx
   11cce:      	movq	%r12, %rsi
   11cd1:      	incq	%r14
   11cd4:      	movq	%rax, %r15
   11cd7:      	jmp	0x11cdf <msx_index_framing+0x11f>
   11cd9:      	movl	$0x1, %r14d
   11cdf:      	cmpq	%rbx, %r14
   11ce2:      	ja	0x11d1a <msx_index_framing+0x15a>
   11ce4:      	subq	%rbx, %rsi
;     let framing_len = original_len - remaining.len();
   11ce7:      	addq	%r14, %rsi
;             *out = FramingOut {
   11cea:      	movq	%rsi, (%rdx)
   11ced:      	movq	%r15, 0x8(%rdx)
   11cf1:      	xorl	%eax, %eax
; }
   11cf3:      	addq	$0x10, %rsp
   11cf7:      	popq	%rbx
   11cf8:      	popq	%r12
   11cfa:      	popq	%r13
   11cfc:      	popq	%r14
   11cfe:      	popq	%r15
   11d00:      	retq
   11d01:      	leaq	-0xcb72(%rip), %rdi     # 0x5196 <writev+0x5196>
   11d08:      	leaq	0x39431(%rip), %rdx     # 0x4b140 <writev+0x4b140>
   11d0f:      	movl	$0x13, %esi
   11d14:      	callq	*0x3ad5e(%rip)          # 0x4ca78 <writev+0x4ca78>
   11d1a:      	leaq	0x39407(%rip), %rcx     # 0x4b128 <writev+0x4b128>
   11d21:      	movq	%r14, %rdi
   11d24:      	movq	%rbx, %rsi
   11d27:      	movq	%rbx, %rdx
   11d2a:      	callq	*0x3ad28(%rip)          # 0x4ca58 <writev+0x4ca58>
   11d30:      	int3
   11d31:      	int3
   11d32:      	int3
   11d33:      	int3
   11d34:      	int3
   11d35:      	int3
   11d36:      	int3
   11d37:      	int3
   11d38:      	int3
   11d39:      	int3
   11d3a:      	int3
   11d3b:      	int3
   11d3c:      	int3
   11d3d:      	int3
   11d3e:      	int3
   11d3f:      	int3
