# Test fixture: crashes on its first call.
        .text
        .globl  msx_vint
        .type   msx_vint,@function
        .p2align 6
msx_vint:
        ud2
        .size   msx_vint, .-msx_vint
        .section .note.GNU-stack,"",@progbits
