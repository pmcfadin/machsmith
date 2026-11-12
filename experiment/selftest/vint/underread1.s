# Sabotage candidate for vint: correct results, but reads the byte at ptr-1.
        .intel_syntax noprefix
        .text
        .p2align 6
        .globl  msx_vint
        .type   msx_vint,@function
msx_vint:
        test    rsi, rsi
        jz      .Lfail
        movzx   r11d, byte ptr [rdi-1]      # under-read
        movzx   eax, byte ptr [rdi]
        mov     ecx, eax
        not     ecx
        shl     ecx, 24
        lzcnt   ecx, ecx            # leading ones of the lead byte (32 when it is 0xFF)
        cmp     ecx, 8
        jbe     .Lextra_ok
        mov     ecx, 8
.Lextra_ok:
        lea     r8, [rcx+1]         # total length
        cmp     rsi, r8
        jb      .Lfail
        mov     r9d, 7
        sub     r9d, ecx            # data bits left in the lead byte
        bzhi    eax, eax, r9d
        cmp     ecx, 8
        jne     .Lfirst_ok
        xor     eax, eax
.Lfirst_ok:
        mov     r10d, 1
.Lloop:
        cmp     r10, r8
        jae     .Ldone
        shl     rax, 8
        movzx   r11d, byte ptr [rdi+r10]
        or      rax, r11
        inc     r10
        jmp     .Lloop
.Ldone:
        mov     [rdx], rax
        mov     rax, r8
        ret
.Lfail:
        xor     eax, eax
        ret
        .size   msx_vint, .-msx_vint
        .section .note.GNU-stack,"",@progbits
