        .text
        .p2align 6
        .globl  msx_vint
        .type   msx_vint, @function
msx_vint:
        testq   %rsi, %rsi
        je      .Lzero
        movzbl  (%rdi), %eax
        testb   %al, %al
        js      .Lmulti
        movq    %rax, (%rdx)
        movl    $1, %eax
        ret
.Lmulti:
        movl    %eax, %ecx
        shll    $24, %ecx
        notl    %ecx
        lzcntl  %ecx, %ecx          # extra, 1..8
        leaq    1(%rcx), %r10       # total
        cmpq    %r10, %rsi
        jb      .Lzero
        cmpq    $8, %rsi
        jb      .Lslow
        cmpl    $8, %ecx
        je      .Lnine
        movbeq  (%rdi), %r8
        leal    (,%r10,8), %r9d
        negl    %r9d
        shrxq   %r9, %r8, %r8       # count mod 64 = 64-8*total
        leal    (,%r10,8), %r9d
        subl    %r10d, %r9d         # 7*total
        bzhiq   %r9, %r8, %r8
        movq    %r8, (%rdx)
        movq    %r10, %rax
        ret
.Lnine:
        movbeq  1(%rdi), %r8
        movq    %r8, (%rdx)
        movl    $9, %eax
        ret
.Lslow:
        movq    %rax, %r8
        movl    $1, %r9d
.Lloop:
        shlq    $8, %r8
        movzbl  (%rdi,%r9), %eax
        orq     %rax, %r8
        incq    %r9
        cmpq    %r10, %r9
        jb      .Lloop
        leal    (,%r10,8), %r9d
        subl    %r10d, %r9d
        bzhiq   %r9, %r8, %r8
        movq    %r8, (%rdx)
        movq    %r10, %rax
        ret
.Lzero:
        xorl    %eax, %eax
        ret
        .size   msx_vint, .-msx_vint
        .section .note.GNU-stack,"",@progbits
