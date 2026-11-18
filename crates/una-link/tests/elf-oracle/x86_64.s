# The per-type fixture of the ELF relocation engine on x86-64 (ADR 0124 rules 24 and 33, step
# 4b): one place, or one sequence, per supported relocation type and per form the engine treats
# differently. tests/elf_oracle.rs assembles it, links it with `ld.lld` (and
# scripts/link-oracle-elf.sh with GNU `ld`) into a static-pie executable, and compares the
# engine's bytes with the linker's at every place (`una_link::elf_oracle`). Discipline: one input
# section per output section name, no `.eh_frame`, every defined symbol named. It is linked with absolute.s.

        # The absolute symbols are defined by absolute.s, another object, so that the assembler
        # keeps the relocations against them instead of writing their values.
        .weak   weak_undef

        .text
        .globl  _start
        .type   _start, @function
_start:
        # R_X86_64_PC32 and R_X86_64_PLT32.
        leaq    data_a(%rip), %rax
        call    func_a
        # R_X86_64_GOTPCREL: a load through the GOT, never relaxed (forced with .reloc).
        .byte   0x48, 0x8b, 0x05
        .reloc  ., R_X86_64_GOTPCREL, data_b-4
        .long   0
        # R_X86_64_GOTPCREL against an absolute symbol (GNU ld 2.42 relaxes it to a PC-relative
        # lea, which it refuses: `ld.lld` alone is its oracle) and an undefined weak one.
        .ifndef GNU_LD
        .byte   0x48, 0x8b, 0x05
        .reloc  ., R_X86_64_GOTPCREL, abs_small-4
        .long   0
        .endif
        .byte   0x48, 0x8b, 0x05
        .reloc  ., R_X86_64_GOTPCREL, weak_undef-4
        .long   0
        # R_X86_64_REX_GOTPCRELX: mov becomes lea, and with r8-r15.
        movq    data_c@GOTPCREL(%rip), %rax
        movq    data_c@GOTPCREL(%rip), %r11
        # R_X86_64_GOTPCRELX: a 32-bit mov becomes lea; call and jmp become direct.
        movl    data_c@GOTPCREL(%rip), %ecx
        call    *func_b@GOTPCREL(%rip)
        jmp     *func_b@GOTPCREL(%rip)
        # R_X86_64_REX_GOTPCRELX that is not a mov (cmp): keeps its GOT slot.
        cmpq    %rax, data_c@GOTPCREL(%rip)
        # R_X86_64_REX_GOTPCRELX against an absolute symbol: keeps its GOT slot. GNU ld 2.42
        # relaxes it to a PC-relative lea of the absolute value, which it then refuses:
        # scripts/link-oracle-elf.sh assembles with GNU_LD defined, so `ld.lld` alone is its
        # oracle.
        .ifndef GNU_LD
        movq    abs_small@GOTPCREL(%rip), %rdx
        .endif
        # R_X86_64_32 and R_X86_64_32S against absolute symbols. GNU ld 2.42 treats an absolute
        # symbol as one that moves with a position-independent executable and refuses them:
        # `ld.lld`, which keeps SHN_ABS absolute as rule 24 does, is their oracle.
        .ifndef GNU_LD
        .byte   0xb8
        .reloc  ., R_X86_64_32, abs_small
        .long   0
        .byte   0x48, 0xc7, 0xc0
        .reloc  ., R_X86_64_32S, abs_neg
        .long   0
        .endif
        # R_X86_64_TLSGD, the general-dynamic sequence calling __tls_get_addr through PLT32.
        .byte   0x66
        leaq    tls_a@tlsgd(%rip), %rdi
        .word   0x6666
        rex64
        call    __tls_get_addr@PLT
        # R_X86_64_TLSGD calling __tls_get_addr through the GOT.
        .byte   0x66
        leaq    tls_b@tlsgd(%rip), %rdi
        .byte   0x66
        rex64
        call    *__tls_get_addr@GOTPCREL(%rip)
        # R_X86_64_TLSLD and R_X86_64_DTPOFF32, through PLT32.
        leaq    tls_c@tlsld(%rip), %rdi
        call    __tls_get_addr@PLT
        movq    tls_c@dtpoff(%rax), %rcx
        leaq    tls_b@dtpoff(%rax), %rcx
        # R_X86_64_TLSLD through the GOT.
        leaq    tls_c@tlsld(%rip), %rdi
        call    *__tls_get_addr@GOTPCREL(%rip)
        # R_X86_64_GOTTPOFF: movq and addq, with low and high registers and %r12.
        movq    tls_a@gottpoff(%rip), %rax
        movq    tls_a@gottpoff(%rip), %r9
        addq    tls_b@gottpoff(%rip), %rcx
        addq    tls_b@gottpoff(%rip), %r10
        addq    tls_b@gottpoff(%rip), %r12
        # R_X86_64_TPOFF32.
        movq    %fs:tls_c@tpoff, %rax
        leaq    tls_a@tpoff+4(%rax), %rdx
        # R_X86_64_PLT32 to an undefined weak function, guarded as crtbeginS.o guards it.
        call    weak_undef@PLT
        ret

        .globl  func_a
        .type   func_a, @function
func_a:
        ret
        .globl  func_b
        .type   func_b, @function
func_b:
        ret
        .globl  __tls_get_addr
        .type   __tls_get_addr, @function
__tls_get_addr:
        ret

        .data
        .globl  data_a
data_a:
        .quad   0
        .globl  data_b
data_b:
        .quad   0
        .globl  data_c
data_c:
        .quad   0
        # R_X86_64_64: a relative fixup, an absolute value, an undefined weak symbol (0).
        .quad   func_a
        .quad   data_b+16
        .reloc  ., R_X86_64_64, abs_small
        .quad   0
        .quad   weak_undef
        # R_X86_64_PC64 and R_X86_64_PC32 in data.
        .quad   func_a - .
        .long   func_b - .
        # R_X86_64_32 against an absolute symbol in data (`ld.lld` alone, as above).
        .ifndef GNU_LD
        .reloc  ., R_X86_64_32, abs_small
        .long   0
        .endif

        .section .data.rel.ro, "aw"
        .globl  relro_a
relro_a:
        .quad   func_b
        .quad   data_c

        .section .tdata, "awT", @progbits
        .globl  tls_a
tls_a:
        .quad   0x1111
        .globl  tls_b
tls_b:
        .quad   0x2222

        .section .tbss, "awT", @nobits
        .globl  tls_c
tls_c:
        .zero   24

        .section .note.GNU-stack, "", @progbits
