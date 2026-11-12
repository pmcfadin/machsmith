	.text
	.globl	fixture
	.type	fixture, @function
fixture:
	leaq	(%rdi,%rsi), %rax
	addq	%rdx, %rax
	addq	%rcx, %rax
	ret
	.section	.note.GNU-stack,"",@progbits
