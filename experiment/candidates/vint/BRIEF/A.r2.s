
../work/build/A/r2/libmsx_kernels.so:	file format elf64-x86-64

Disassembly of section .text:

0000000000011f40 <msx_vint>:
   11f40:      	testq	%rsi, %rsi
   11f43:      	je	0x11fc3 <msx_vint+0x83>
   11f45:      	pushq	%r15
   11f47:      	pushq	%r14
   11f49:      	pushq	%rbx
   11f4a:      	subq	$0x10, %rsp
;     let first = *input.first().ok_or(VIntError::Truncated)?;
   11f4e:      	movzbl	(%rdi), %ebx
   11f51:      	movl	%ebx, %eax
   11f53:      	shll	$0x18, %eax
   11f56:      	notl	%eax
   11f58:      	lzcntl	%eax, %eax
;     if input.len() < total {
   11f5c:      	cmpq	%rax, %rsi
   11f5f:      	jbe	0x11fc6 <msx_vint+0x86>
;     if extra == 0 {
   11f61:      	testq	%rax, %rax
   11f64:      	je	0x11fca <msx_vint+0x8a>
   11f66:      	leaq	0x8(%rsp), %rcx
   11f6b:      	incq	%rdi
   11f6e:      	movq	%rdx, %r14
;     let mut be = [0u8; 8];
   11f71:      	movq	$0x0, 0x8(%rsp)
   11f7a:      	movq	%rax, %r15
   11f7d:      	movq	%rax, %rdx
   11f80:      	subq	%rax, %rcx
   11f83:      	movq	%rdi, %rsi
   11f86:      	addq	$0x8, %rcx
   11f8a:      	movq	%rcx, %rdi
   11f8d:      	callq	*0x3aa7d(%rip)          # 0x4ca10 <writev+0x4ca10>
   11f93:      	movbeq	0x8(%rsp), %rcx
;     let value = if extra == 8 {
   11f9a:      	cmpl	$0xff, %ebx
   11fa0:      	je	0x11fd1 <msx_vint+0x91>
   11fa2:      	movb	$0x7, %dl
   11fa4:      	movq	%r15, %rax
;         ((first as u64 & mask) << (extra * 8)) | tail
   11fa7:      	leal	(,%r15,8), %esi
;         let mask = (1u64 << data_bits_first) - 1;
   11faf:      	subb	%al, %dl
;         ((first as u64 & mask) << (extra * 8)) | tail
   11fb1:      	bzhil	%edx, %ebx, %edx
   11fb6:      	shlxq	%rsi, %rdx, %rdx
   11fbb:      	orq	%rdx, %rcx
   11fbe:      	movq	%r14, %rdx
   11fc1:      	jmp	0x11fd7 <msx_vint+0x97>
   11fc3:      	xorl	%eax, %eax
; }
   11fc5:      	retq
   11fc6:      	xorl	%eax, %eax
;     if input.len() < total {
   11fc8:      	jmp	0x11fe0 <msx_vint+0xa0>
   11fca:      	movl	$0x1, %eax
   11fcf:      	jmp	0x11fdd <msx_vint+0x9d>
   11fd1:      	movq	%r14, %rdx
   11fd4:      	movq	%r15, %rax
   11fd7:      	incq	%rax
   11fda:      	movq	%rcx, %rbx
;             *out = value;
   11fdd:      	movq	%rbx, (%rdx)
   11fe0:      	addq	$0x10, %rsp
   11fe4:      	popq	%rbx
   11fe5:      	popq	%r14
   11fe7:      	popq	%r15
; }
   11fe9:      	retq
   11fea:      	int3
   11feb:      	int3
   11fec:      	int3
   11fed:      	int3
   11fee:      	int3
   11fef:      	int3
   11ff0:      	int3
   11ff1:      	int3
   11ff2:      	int3
   11ff3:      	int3
   11ff4:      	int3
   11ff5:      	int3
   11ff6:      	int3
   11ff7:      	int3
   11ff8:      	int3
   11ff9:      	int3
   11ffa:      	int3
   11ffb:      	int3
   11ffc:      	int3
   11ffd:      	int3
   11ffe:      	int3
   11fff:      	int3
