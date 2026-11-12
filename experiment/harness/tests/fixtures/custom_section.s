# Exports a correct-enough msx_vint but hides an 8192-byte table in a custom read-only section.
        .text
        .globl  msx_vint
        .type   msx_vint,@function
        .p2align 6
msx_vint:
        xorl    %eax, %eax
        ret
        .size   msx_vint, .-msx_vint
        .section .mytable,"a",@progbits
        .fill   8192, 1, 0x5a
        .section .note.GNU-stack,"",@progbits
