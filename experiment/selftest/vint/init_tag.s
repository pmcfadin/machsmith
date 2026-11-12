.text
.globl msx_vint
.type msx_vint,@function
.p2align 6
msx_vint:
  xorl %eax,%eax
  ret
.size msx_vint,.-msx_vint
.globl _init
.type _init,@function
_init:
  ret
.size _init,.-_init
.section .note.GNU-stack,"",@progbits
