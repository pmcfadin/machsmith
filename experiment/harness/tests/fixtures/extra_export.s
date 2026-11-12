# Exports a second global function next to msx_vint.
        .text
        .globl  msx_vint
        .type   msx_vint,@function
        .p2align 6
msx_vint:
        xorl    %eax, %eax
        ret
        .size   msx_vint, .-msx_vint
        .globl  helper
        .type   helper,@function
helper:
        ret
        .size   helper, .-helper
        .section .note.GNU-stack,"",@progbits
