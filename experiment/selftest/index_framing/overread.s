# Sabotage candidate for index_framing: correct results, but loads 8 bytes at ptr+len (past the end) whenever len >= 1.
        .intel_syntax noprefix
        .text
        .p2align 6
        .globl  msx_index_framing
        .type   msx_index_framing,@function
msx_index_framing:
        cmp     rsi, 2
        jb      .Lfail
        test    rsi, rsi
        jz      .Lskip
        mov     r10, [rdi+rsi]          # over-read, result unused
.Lskip:
        movzx   eax, word ptr [rdi]
        rol     ax, 8                   # key_len, big endian
        movzx   eax, ax
        lea     r8, [rax+2]             # pos = 2 + key_len
        mov     r11d, 2                 # two vints follow the key
.Lvint:
        cmp     r8, rsi
        jae     .Lfail                  # key or vint lead byte past the end
        movzx   eax, byte ptr [rdi+r8]
        mov     ecx, eax
        not     ecx
        shl     ecx, 24
        lzcnt   ecx, ecx                # leading ones of the lead byte (32 when it is 0xFF)
        cmp     ecx, 8
        jbe     .Lextra_ok
        mov     ecx, 8
.Lextra_ok:
        lea     r10, [r8+rcx+1]         # end of this vint
        cmp     r10, rsi
        ja      .Lfail
        mov     r9d, 7
        sub     r9d, ecx
        bzhi    eax, eax, r9d
        cmp     ecx, 8
        jne     .Lfirst_ok
        xor     eax, eax
.Lfirst_ok:
        inc     r8
.Lloop:
        cmp     r8, r10
        jae     .Lvint_done
        shl     rax, 8
        movzx   r9d, byte ptr [rdi+r8]
        or      rax, r9
        inc     r8
        jmp     .Lloop
.Lvint_done:
        dec     r11d
        jnz     .Lvint
        mov     [rdx], r10              # framing_len
        mov     [rdx+8], rax            # promoted_len
        xor     eax, eax
        ret
.Lfail:
        mov     eax, 24                 # 1 + ErrorKind::Eof (23)
        ret
        .size   msx_index_framing, .-msx_index_framing
        .section .note.GNU-stack,"",@progbits
