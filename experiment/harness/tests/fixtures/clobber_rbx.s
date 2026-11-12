	.text
	.globl	fixture
	.type	fixture, @function
fixture:
	xorl	%ebx, %ebx
	xorl	%eax, %eax
	ret
	.section	.note.GNU-stack,"",@progbits
