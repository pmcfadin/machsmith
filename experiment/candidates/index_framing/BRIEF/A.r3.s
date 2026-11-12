
../work/build/A/r3/libmsx_kernels.so:	file format elf64-x86-64

Disassembly of section .text:

0000000000011b80 <msx_index_framing>:
; pub unsafe extern "C" fn msx_index_framing(ptr: *const u8, len: usize, out: *mut FramingOut) -> u32 {
   11b80:      	pushq	%r15
   11b82:      	pushq	%r14
   11b84:      	pushq	%r13
   11b86:      	pushq	%r12
   11b88:      	pushq	%rbx
   11b89:      	subq	$0x10, %rsp
   11b8d:      	movl	$0x18, %eax
;   if input.input_len() < bound {
   11b92:      	cmpq	$0x2, %rsi
   11b96:      	jb	0x11c26 <msx_index_framing+0xa6>
;       res = (res << 8) + byte as u16;
   11b9c:      	movbew	(%rdi), %cx
   11ba1:      	leaq	-0x2(%rsi), %rbx
   11ba5:      	movq	%rbx, %r8
;       res = (res << 8) + byte as u16;
   11ba8:      	movzwl	%cx, %ecx
;     if self.len() >= count {
   11bab:      	cmpq	%rcx, %rbx
   11bae:      	jb	0x11bb3 <msx_index_framing+0x33>
   11bb0:      	xorl	%r8d, %r8d
;     if self.len() >= count {
   11bb3:      	cmpq	%rcx, %rbx
;   move |i: Input| match i.slice_index(c) {
   11bb6:      	jb	0x11c26 <msx_index_framing+0xa6>
   11bb8:      	subq	%r8, %rcx
   11bbb:      	cmpq	%rcx, %rbx
   11bbe:      	jb	0x11c96 <msx_index_framing+0x116>
   11bc4:      	subq	%rcx, %rbx
   11bc7:      	je	0x11c26 <msx_index_framing+0xa6>
   11bc9:      	addq	$0x2, %rdi
   11bcd:      	addq	%rcx, %rdi
   11bd0:      	movzbl	(%rdi), %ecx
   11bd3:      	shll	$0x18, %ecx
   11bd6:      	notl	%ecx
   11bd8:      	lzcntl	%ecx, %ecx
;     if input.len() < total {
   11bdc:      	cmpq	%rcx, %rbx
   11bdf:      	jbe	0x11c26 <msx_index_framing+0xa6>
;     let total = extra + 1;
   11be1:      	leaq	0x1(%rcx), %r8
   11be5:      	subq	%r8, %rbx
   11be8:      	je	0x11c26 <msx_index_framing+0xa6>
   11bea:      	leaq	0x1(%rdi,%rcx), %rcx
;     let first = *input.first().ok_or(VIntError::Truncated)?;
   11bef:      	movzbl	(%rcx), %r15d
   11bf3:      	movl	%r15d, %edi
   11bf6:      	shll	$0x18, %edi
   11bf9:      	notl	%edi
   11bfb:      	lzcntl	%edi, %edi
;     if input.len() < total {
   11bff:      	cmpq	%rdi, %rbx
   11c02:      	jbe	0x11c26 <msx_index_framing+0xa6>
;     if extra == 0 {
   11c04:      	testq	%rdi, %rdi
   11c07:      	jne	0x11c34 <msx_index_framing+0xb4>
   11c09:      	movl	$0x1, %edi
   11c0e:      	cmpq	%rbx, %rdi
   11c11:      	ja	0x11caf <msx_index_framing+0x12f>
   11c17:      	subq	%rbx, %rsi
   11c1a:      	xorl	%eax, %eax
;     let framing_len = original_len - remaining.len();
   11c1c:      	addq	%rdi, %rsi
;             *out = FramingOut {
   11c1f:      	movq	%rsi, (%rdx)
   11c22:      	movq	%r15, 0x8(%rdx)
; }
   11c26:      	addq	$0x10, %rsp
   11c2a:      	popq	%rbx
   11c2b:      	popq	%r12
   11c2d:      	popq	%r13
   11c2f:      	popq	%r14
   11c31:      	popq	%r15
   11c33:      	retq
   11c34:      	leaq	0x8(%rsp), %rax
   11c39:      	incq	%rcx
;     let mut be = [0u8; 8];
   11c3c:      	movq	$0x0, 0x8(%rsp)
   11c45:      	subq	%rdi, %rax
   11c48:      	addq	$0x8, %rax
   11c4c:      	cmpq	$0x1, %rdi
   11c50:      	jne	0x11c89 <msx_index_framing+0x109>
   11c52:      	movzbl	(%rcx), %ecx
   11c55:      	movb	%cl, (%rax)
   11c57:      	movbeq	0x8(%rsp), %rax
;     let value = if extra == 8 {
   11c5e:      	cmpl	$0xff, %r15d
   11c65:      	je	0x11c81 <msx_index_framing+0x101>
   11c67:      	movb	$0x7, %cl
;         ((first as u64 & mask) << (extra * 8)) | tail
   11c69:      	leal	(,%rdi,8), %r8d
;         let mask = (1u64 << data_bits_first) - 1;
   11c71:      	subb	%dil, %cl
;         ((first as u64 & mask) << (extra * 8)) | tail
   11c74:      	bzhil	%ecx, %r15d, %ecx
   11c79:      	shlxq	%r8, %rcx, %rcx
   11c7e:      	orq	%rcx, %rax
   11c81:      	incq	%rdi
   11c84:      	movq	%rax, %r15
   11c87:      	jmp	0x11c0e <msx_index_framing+0x8e>
   11c89:      	cmpl	$0x2, %edi
   11c8c:      	jne	0x11cc2 <msx_index_framing+0x142>
   11c8e:      	movzwl	(%rcx), %ecx
   11c91:      	movw	%cx, (%rax)
   11c94:      	jmp	0x11c57 <msx_index_framing+0xd7>
   11c96:      	leaq	-0xcaf7(%rip), %rdi     # 0x51a6 <writev+0x51a6>
   11c9d:      	leaq	0x3951c(%rip), %rdx     # 0x4b1c0 <writev+0x4b1c0>
   11ca4:      	movl	$0x13, %esi
   11ca9:      	callq	*0x3ae79(%rip)          # 0x4cb28 <writev+0x4cb28>
   11caf:      	leaq	0x394f2(%rip), %rcx     # 0x4b1a8 <writev+0x4b1a8>
   11cb6:      	movq	%rbx, %rsi
   11cb9:      	movq	%rbx, %rdx
   11cbc:      	callq	*0x3ae16(%rip)          # 0x4cad8 <writev+0x4cad8>
   11cc2:      	movq	%rdi, %r14
   11cc5:      	movq	%rsi, %r12
   11cc8:      	movq	%rdx, %r13
   11ccb:      	movq	%rax, %rdi
   11cce:      	movq	%rcx, %rsi
   11cd1:      	movq	%r14, %rdx
   11cd4:      	callq	*0x3ae06(%rip)          # 0x4cae0 <writev+0x4cae0>
   11cda:      	movq	%r14, %rdi
   11cdd:      	movq	%r12, %rsi
   11ce0:      	movq	%r13, %rdx
   11ce3:      	jmp	0x11c57 <msx_index_framing+0xd7>
   11ce8:      	int3
   11ce9:      	int3
   11cea:      	int3
   11ceb:      	int3
   11cec:      	int3
   11ced:      	int3
   11cee:      	int3
   11cef:      	int3
   11cf0:      	int3
   11cf1:      	int3
   11cf2:      	int3
   11cf3:      	int3
   11cf4:      	int3
   11cf5:      	int3
   11cf6:      	int3
   11cf7:      	int3
   11cf8:      	int3
   11cf9:      	int3
   11cfa:      	int3
   11cfb:      	int3
   11cfc:      	int3
   11cfd:      	int3
   11cfe:      	int3
   11cff:      	int3
