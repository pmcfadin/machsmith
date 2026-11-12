	.text
	.p2align 6
	.globl msx_index_framing
	.type msx_index_framing, @function
msx_index_framing:
	cmpq	$2, %rsi
	jb	.Lerr
	movbew	(%rdi), %cx
	movzwl	%cx, %ecx
	leaq	2(%rcx), %r8
	cmpq	%rsi, %r8
	jae	.Lerr
	movzbl	(%rdi,%r8), %eax
	incq	%r8
	testb	%al, %al
	js	.Lv1slow
.Lv2:
	cmpq	%rsi, %r8
	jae	.Lerr
	movzbl	(%rdi,%r8), %eax
	incq	%r8
	testb	%al, %al
	js	.Lv2slow
.Lok:
	movq	%r8, (%rdx)
	movq	%rax, 8(%rdx)
	xorl	%eax, %eax
	ret
.Lerr:
	movl	$24, %eax
	ret
.Lv1slow:
	movl	%eax, %ecx
	shll	$24, %ecx
	notl	%ecx
	lzcntl	%ecx, %ecx
	addq	%rcx, %r8
	cmpq	%rsi, %r8
	jbe	.Lv2
	jmp	.Lerr
.Lv2slow:
	movl	%eax, %ecx
	shll	$24, %ecx
	notl	%ecx
	lzcntl	%ecx, %ecx
	leaq	(%r8,%rcx), %r9
	cmpq	%rsi, %r9
	ja	.Lerr
	xorl	%r10d, %r10d
	cmpl	$8, %ecx
	je	.Lloop
	movl	$7, %r11d
	subl	%ecx, %r11d
	bzhil	%r11d, %eax, %r10d
.Lloop:
	movzbl	(%rdi,%r8), %r11d
	shlq	$8, %r10
	orq	%r11, %r10
	incq	%r8
	decl	%ecx
	jnz	.Lloop
	movq	%r10, %rax
	jmp	.Lok
	.size	msx_index_framing, .-msx_index_framing
	.section .note.GNU-stack,"",@progbits
