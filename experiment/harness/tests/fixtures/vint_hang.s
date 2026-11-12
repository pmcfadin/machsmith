# Test fixture: never returns.
        .text
        .globl  msx_vint
        .type   msx_vint,@function
        .p2align 6
msx_vint:
.Lspin:
        jmp     .Lspin
        .size   msx_vint, .-msx_vint
        .section .note.GNU-stack,"",@progbits
