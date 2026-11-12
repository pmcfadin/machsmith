	.text
	.globl	fixture
	.type	fixture, @function
fixture:
	std
	xorl	%eax, %eax
	ret
	.section	.note.GNU-stack,"",@progbits
