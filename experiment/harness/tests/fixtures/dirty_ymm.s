	.text
	.globl	fixture
	.type	fixture, @function
fixture:
	vpcmpeqd	%ymm0, %ymm0, %ymm0
	xorl	%eax, %eax
	ret
	.section	.note.GNU-stack,"",@progbits
