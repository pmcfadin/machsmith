
../work/build/A/r3/libmsx_kernels.so:	file format elf64-x86-64

Disassembly of section .text:

0000000000011f80 <msx_vint>:
   11f80:      	testq	%rsi, %rsi
   11f83:      	je	0x1204e <msx_vint+0xce>
   11f89:      	pushq	%r15
   11f8b:      	pushq	%r14
   11f8d:      	pushq	%rbx
   11f8e:      	subq	$0x10, %rsp
;     let first = *input.first().ok_or(VIntError::Truncated)?;
   11f92:      	movzbl	(%rdi), %ebx
   11f95:      	movl	%ebx, %eax
   11f97:      	shll	$0x18, %eax
   11f9a:      	notl	%eax
   11f9c:      	lzcntl	%eax, %eax
;     if input.len() < total {
   11fa0:      	cmpq	%rax, %rsi
   11fa3:      	jbe	0x12051 <msx_vint+0xd1>
;     if extra == 0 {
   11fa9:      	testq	%rax, %rax
   11fac:      	jne	0x11fc0 <msx_vint+0x40>
   11fae:      	movl	$0x1, %eax
;             *out = value;
   11fb3:      	movq	%rbx, (%rdx)
   11fb6:      	addq	$0x10, %rsp
   11fba:      	popq	%rbx
   11fbb:      	popq	%r14
   11fbd:      	popq	%r15
; }
   11fbf:      	retq
   11fc0:      	incq	%rdi
;     let mut be = [0u8; 8];
   11fc3:      	movq	$0x0, 0x8(%rsp)
   11fcc:      	movq	%rdi, %rsi
   11fcf:      	leaq	0x8(%rsp), %rdi
   11fd4:      	subq	%rax, %rdi
   11fd7:      	addq	$0x8, %rdi
   11fdb:      	cmpq	$0x2, %rax
   11fdf:      	je	0x11fed <msx_vint+0x6d>
   11fe1:      	cmpl	$0x1, %eax
   11fe4:      	jne	0x12024 <msx_vint+0xa4>
   11fe6:      	movzbl	(%rsi), %ecx
   11fe9:      	movb	%cl, (%rdi)
   11feb:      	jmp	0x11ff3 <msx_vint+0x73>
   11fed:      	movzwl	(%rsi), %ecx
   11ff0:      	movw	%cx, (%rdi)
   11ff3:      	movbeq	0x8(%rsp), %rcx
;     let value = if extra == 8 {
   11ffa:      	cmpl	$0xff, %ebx
   12000:      	je	0x1201c <msx_vint+0x9c>
   12002:      	movb	$0x7, %sil
;         ((first as u64 & mask) << (extra * 8)) | tail
   12005:      	leal	(,%rax,8), %edi
;         let mask = (1u64 << data_bits_first) - 1;
   1200c:      	subb	%al, %sil
;         ((first as u64 & mask) << (extra * 8)) | tail
   1200f:      	bzhil	%esi, %ebx, %esi
   12014:      	shlxq	%rdi, %rsi, %rsi
   12019:      	orq	%rsi, %rcx
   1201c:      	incq	%rax
   1201f:      	movq	%rcx, %rbx
   12022:      	jmp	0x11fb3 <msx_vint+0x33>
   12024:      	cmpl	$0x6, %eax
   12027:      	jne	0x12037 <msx_vint+0xb7>
   12029:      	movzwl	0x4(%rsi), %ecx
   1202d:      	movw	%cx, 0x4(%rdi)
   12031:      	movl	(%rsi), %ecx
   12033:      	movl	%ecx, (%rdi)
   12035:      	jmp	0x11ff3 <msx_vint+0x73>
   12037:      	movq	%rdx, %r14
   1203a:      	movq	%rax, %rdx
   1203d:      	movq	%rax, %r15
   12040:      	callq	*0x3aa9a(%rip)          # 0x4cae0 <writev+0x4cae0>
   12046:      	movq	%r15, %rax
   12049:      	movq	%r14, %rdx
   1204c:      	jmp	0x11ff3 <msx_vint+0x73>
   1204e:      	xorl	%eax, %eax
; }
   12050:      	retq
   12051:      	xorl	%eax, %eax
;     if input.len() < total {
   12053:      	jmp	0x11fb6 <msx_vint+0x36>
   12058:      	int3
   12059:      	int3
   1205a:      	int3
   1205b:      	int3
   1205c:      	int3
   1205d:      	int3
   1205e:      	int3
   1205f:      	int3
   12060:      	int3
   12061:      	int3
   12062:      	int3
   12063:      	int3
   12064:      	int3
   12065:      	int3
   12066:      	int3
   12067:      	int3
   12068:      	int3
   12069:      	int3
   1206a:      	int3
   1206b:      	int3
   1206c:      	int3
   1206d:      	int3
   1206e:      	int3
   1206f:      	int3
   12070:      	int3
   12071:      	int3
   12072:      	int3
   12073:      	int3
   12074:      	int3
   12075:      	int3
   12076:      	int3
   12077:      	int3
   12078:      	int3
   12079:      	int3
   1207a:      	int3
   1207b:      	int3
   1207c:      	int3
   1207d:      	int3
   1207e:      	int3
   1207f:      	int3
