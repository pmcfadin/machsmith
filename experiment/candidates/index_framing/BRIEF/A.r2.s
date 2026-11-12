
../work/build/A/r2/libmsx_kernels.so:	file format elf64-x86-64

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
   11bd6:      	jb	0x11cd6 <msx_index_framing+0x116>
;       res = (res << 8) + byte as u16;
   11bdc:      	movbew	(%rdi), %cx
   11be1:      	leaq	-0x2(%rsi), %rbx
   11be5:      	xorl	%r8d, %r8d
;       res = (res << 8) + byte as u16;
   11be8:      	movzwl	%cx, %ecx
;     if self.len() >= count {
   11beb:      	cmpq	%rcx, %rbx
   11bee:      	cmovbq	%rbx, %r8
;   move |i: Input| match i.slice_index(c) {
   11bf2:      	jb	0x11cd6 <msx_index_framing+0x116>
   11bf8:      	subq	%r8, %rcx
   11bfb:      	cmpq	%rcx, %rbx
   11bfe:      	jb	0x11ce4 <msx_index_framing+0x124>
   11c04:      	subq	%rcx, %rbx
   11c07:      	je	0x11cd6 <msx_index_framing+0x116>
   11c0d:      	addq	$0x2, %rdi
   11c11:      	addq	%rcx, %rdi
   11c14:      	movzbl	(%rdi), %ecx
   11c17:      	shll	$0x18, %ecx
   11c1a:      	notl	%ecx
   11c1c:      	lzcntl	%ecx, %ecx
;     if input.len() < total {
   11c20:      	cmpq	%rcx, %rbx
   11c23:      	jbe	0x11cd6 <msx_index_framing+0x116>
;     let total = extra + 1;
   11c29:      	leaq	0x1(%rcx), %r8
   11c2d:      	subq	%r8, %rbx
   11c30:      	je	0x11cd6 <msx_index_framing+0x116>
   11c36:      	leaq	0x1(%rdi,%rcx), %rcx
;     let first = *input.first().ok_or(VIntError::Truncated)?;
   11c3b:      	movzbl	(%rcx), %r15d
   11c3f:      	movl	%r15d, %edi
   11c42:      	shll	$0x18, %edi
   11c45:      	notl	%edi
   11c47:      	lzcntl	%edi, %r14d
;     if input.len() < total {
   11c4c:      	cmpq	%r14, %rbx
   11c4f:      	jbe	0x11cd6 <msx_index_framing+0x116>
;     if extra == 0 {
   11c55:      	testq	%r14, %r14
   11c58:      	je	0x11cbc <msx_index_framing+0xfc>
   11c5a:      	leaq	0x8(%rsp), %rdi
   11c5f:      	incq	%rcx
   11c62:      	movq	%rsi, %r12
   11c65:      	movq	%rdx, %r13
;     let mut be = [0u8; 8];
   11c68:      	movq	$0x0, 0x8(%rsp)
   11c71:      	movq	%r14, %rdx
   11c74:      	subq	%r14, %rdi
   11c77:      	movq	%rcx, %rsi
   11c7a:      	addq	$0x8, %rdi
   11c7e:      	callq	*0x3ad8c(%rip)          # 0x4ca10 <writev+0x4ca10>
   11c84:      	movbeq	0x8(%rsp), %rax
;     let value = if extra == 8 {
   11c8b:      	cmpl	$0xff, %r15d
   11c92:      	je	0x11cae <msx_index_framing+0xee>
   11c94:      	movb	$0x7, %cl
;         ((first as u64 & mask) << (extra * 8)) | tail
   11c96:      	leal	(,%r14,8), %edx
;         let mask = (1u64 << data_bits_first) - 1;
   11c9e:      	subb	%r14b, %cl
;         ((first as u64 & mask) << (extra * 8)) | tail
   11ca1:      	bzhil	%ecx, %r15d, %ecx
   11ca6:      	shlxq	%rdx, %rcx, %rcx
   11cab:      	orq	%rcx, %rax
   11cae:      	incq	%r14
   11cb1:      	movq	%r13, %rdx
   11cb4:      	movq	%r12, %rsi
   11cb7:      	movq	%rax, %r15
   11cba:      	jmp	0x11cc2 <msx_index_framing+0x102>
   11cbc:      	movl	$0x1, %r14d
   11cc2:      	cmpq	%rbx, %r14
   11cc5:      	ja	0x11cfd <msx_index_framing+0x13d>
   11cc7:      	subq	%rbx, %rsi
   11cca:      	xorl	%eax, %eax
;     let framing_len = original_len - remaining.len();
   11ccc:      	addq	%r14, %rsi
;             *out = FramingOut {
   11ccf:      	movq	%rsi, (%rdx)
   11cd2:      	movq	%r15, 0x8(%rdx)
; }
   11cd6:      	addq	$0x10, %rsp
   11cda:      	popq	%rbx
   11cdb:      	popq	%r12
   11cdd:      	popq	%r13
   11cdf:      	popq	%r14
   11ce1:      	popq	%r15
   11ce3:      	retq
   11ce4:      	leaq	-0xcb55(%rip), %rdi     # 0x5196 <writev+0x5196>
   11ceb:      	leaq	0x3940e(%rip), %rdx     # 0x4b100 <writev+0x4b100>
   11cf2:      	movl	$0x13, %esi
   11cf7:      	callq	*0x3ad3b(%rip)          # 0x4ca38 <writev+0x4ca38>
   11cfd:      	leaq	0x393e4(%rip), %rcx     # 0x4b0e8 <writev+0x4b0e8>
   11d04:      	movq	%r14, %rdi
   11d07:      	movq	%rbx, %rsi
   11d0a:      	movq	%rbx, %rdx
   11d0d:      	callq	*0x3ad05(%rip)          # 0x4ca18 <writev+0x4ca18>
   11d13:      	int3
   11d14:      	int3
   11d15:      	int3
   11d16:      	int3
   11d17:      	int3
   11d18:      	int3
   11d19:      	int3
   11d1a:      	int3
   11d1b:      	int3
   11d1c:      	int3
   11d1d:      	int3
   11d1e:      	int3
   11d1f:      	int3
   11d20:      	int3
   11d21:      	int3
   11d22:      	int3
   11d23:      	int3
   11d24:      	int3
   11d25:      	int3
   11d26:      	int3
   11d27:      	int3
   11d28:      	int3
   11d29:      	int3
   11d2a:      	int3
   11d2b:      	int3
   11d2c:      	int3
   11d2d:      	int3
   11d2e:      	int3
   11d2f:      	int3
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
