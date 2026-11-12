
../work/build/A/r1/libmsx_kernels.so:	file format elf64-x86-64

Disassembly of section .text:

0000000000011f40 <msx_vint>:
   11f40:      	testq	%rsi, %rsi
   11f43:      	je	0x11fd3 <msx_vint+0x93>
   11f49:      	pushq	%r15
   11f4b:      	pushq	%r14
   11f4d:      	pushq	%rbx
   11f4e:      	subq	$0x10, %rsp
;     let first = *input.first().ok_or(VIntError::Truncated)?;
   11f52:      	movzbl	(%rdi), %ebx
   11f55:      	movl	%ebx, %eax
   11f57:      	notb	%al
   11f59:      	movzbl	%al, %ecx
   11f5c:      	movl	$0xf, %eax
   11f61:      	bsrl	%ecx, %eax
   11f64:      	xorl	$0x7, %eax
;     if input.len() < total {
   11f67:      	cmpq	%rax, %rsi
   11f6a:      	jbe	0x11fd6 <msx_vint+0x96>
;     if extra == 0 {
   11f6c:      	testq	%rax, %rax
   11f6f:      	je	0x11fda <msx_vint+0x9a>
   11f71:      	movq	%rdx, %r14
;     let mut be = [0u8; 8];
   11f74:      	movq	$0x0, 0x8(%rsp)
   11f7d:      	incq	%rdi
   11f80:      	leaq	0x8(%rsp), %rcx
   11f85:      	subq	%rax, %rcx
   11f88:      	addq	$0x8, %rcx
   11f8c:      	movq	%rdi, %rsi
   11f8f:      	movq	%rcx, %rdi
   11f92:      	movq	%rax, %r15
   11f95:      	movq	%rax, %rdx
   11f98:      	callq	*0x3aab2(%rip)          # 0x4ca50 <writev+0x4ca50>
;     let tail = u64::from_be_bytes(be);
   11f9e:      	movq	0x8(%rsp), %rsi
   11fa3:      	bswapq	%rsi
;     let value = if extra == 8 {
   11fa6:      	cmpl	$0xff, %ebx
   11fac:      	je	0x11fe1 <msx_vint+0xa1>
   11fae:      	movb	$0x7, %cl
   11fb0:      	movq	%r15, %rax
;         let mask = (1u64 << data_bits_first) - 1;
   11fb3:      	subb	%al, %cl
   11fb5:      	movl	$0xffffffff, %edx       # imm = 0xFFFFFFFF
   11fba:      	shll	%cl, %edx
   11fbc:      	notl	%edx
;         ((first as u64 & mask) << (extra * 8)) | tail
   11fbe:      	andl	%edx, %ebx
   11fc0:      	leal	(,%r15,8), %ecx
   11fc8:      	shlq	%cl, %rbx
   11fcb:      	orq	%rbx, %rsi
   11fce:      	movq	%r14, %rdx
   11fd1:      	jmp	0x11fe7 <msx_vint+0xa7>
   11fd3:      	xorl	%eax, %eax
; }
   11fd5:      	retq
   11fd6:      	xorl	%eax, %eax
;     if input.len() < total {
   11fd8:      	jmp	0x11ff0 <msx_vint+0xb0>
   11fda:      	movl	$0x1, %eax
   11fdf:      	jmp	0x11fed <msx_vint+0xad>
   11fe1:      	movq	%r14, %rdx
   11fe4:      	movq	%r15, %rax
   11fe7:      	incq	%rax
   11fea:      	movq	%rsi, %rbx
;             *out = value;
   11fed:      	movq	%rbx, (%rdx)
   11ff0:      	addq	$0x10, %rsp
   11ff4:      	popq	%rbx
   11ff5:      	popq	%r14
   11ff7:      	popq	%r15
; }
   11ff9:      	retq
   11ffa:      	int3
   11ffb:      	int3
   11ffc:      	int3
   11ffd:      	int3
   11ffe:      	int3
   11fff:      	int3
