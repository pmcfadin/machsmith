	.text
	.globl	fixture
	.type	fixture, @function
fixture:
	popq	%r11
	addq	$8, %rsp
	xorl	%eax, %eax
	jmp	*%r11
	.section	.note.GNU-stack,"",@progbits
