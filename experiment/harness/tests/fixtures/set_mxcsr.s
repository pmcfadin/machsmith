	.text
	.globl	fixture
	.type	fixture, @function
fixture:
	subq	$8, %rsp
	stmxcsr	(%rsp)
	orl	$0x2000, (%rsp)
	ldmxcsr	(%rsp)
	addq	$8, %rsp
	xorl	%eax, %eax
	ret
	.section	.note.GNU-stack,"",@progbits
