// The per-type fixture of the ELF relocation engine on arm64 (ADR 0124 rules 24 and 33, step
// 4b): one place, or one sequence, per supported relocation type and per form the engine treats
// differently. tests/elf_oracle.rs assembles it, links it with `ld.lld` (and
// scripts/link-oracle-elf.sh with GNU `ld`) into a static-pie executable, and compares the
// engine's bytes with the linker's at every place (`una_link::elf_oracle`). Discipline: one input
// section per output section name, no `.eh_frame`, every defined symbol named. It is linked with absolute.s.

        // The absolute symbols are defined by absolute.s, another object, so that the assembler
        // keeps the relocations against them instead of writing their values.
        .weak   weak_undef

        .text
        .globl  _start
        .type   _start, %function
_start:
        // R_AARCH64_ADR_PREL_LO21, ADR_PREL_PG_HI21 and ADD_ABS_LO12_NC.
        adr     x0, data_a
        adrp    x0, data_a
        add     x0, x0, :lo12:data_a
        // R_AARCH64_LDST8 to LDST128_ABS_LO12_NC.
        adrp    x1, byte_a
        ldrb    w2, [x1, :lo12:byte_a]
        ldrh    w2, [x1, :lo12:half_a]
        ldr     w2, [x1, :lo12:word_a]
        ldr     x2, [x1, :lo12:dword_a]
        ldr     q2, [x1, :lo12:quad_a]
        strb    w2, [x1, :lo12:byte_a]
        // R_AARCH64_CALL26, JUMP26, CONDBR19 (b.cond and cbz) and TSTBR14.
        bl      func_a
        b.eq    func_a
        cbz     x0, func_a
        tbnz    x0, #3, func_a
        // R_AARCH64_ADR_GOT_PAGE and LD64_GOT_LO12_NC: an address, an absolute symbol and an
        // undefined weak one.
        adrp    x3, :got:data_b
        ldr     x3, [x3, :got_lo12:data_b]
        .reloc  ., R_AARCH64_ADR_GOT_PAGE, abs_small
        .inst   0x90000003      // adrp x3, 0
        .reloc  ., R_AARCH64_LD64_GOT_LO12_NC, abs_small
        .inst   0xf9400063      // ldr x3, [x3]
        adrp    x3, :got:weak_undef
        ldr     x3, [x3, :got_lo12:weak_undef]
        // GNU ld 2.42 refuses MOVW_UABS against an absolute symbol in a position-independent
        // executable, and does not know LDST128_TPREL: scripts/link-oracle-elf.sh assembles
        // with GNU_LD defined, so `ld.lld` alone is their oracle.
        .ifndef GNU_LD
        // R_AARCH64_MOVW_UABS_G0 to G3 against absolute symbols (forced with .reloc: the
        // assembler would write the values itself).
        .reloc  ., R_AARCH64_MOVW_UABS_G3, abs_big
        .inst   0xd2e00004      // movz x4, #0, lsl #48
        .reloc  ., R_AARCH64_MOVW_UABS_G2_NC, abs_big
        .inst   0xf2c00004      // movk x4, #0, lsl #32
        .reloc  ., R_AARCH64_MOVW_UABS_G1_NC, abs_big
        .inst   0xf2a00004      // movk x4, #0, lsl #16
        .reloc  ., R_AARCH64_MOVW_UABS_G0_NC, abs_big
        .inst   0xf2800004      // movk x4, #0
        .reloc  ., R_AARCH64_MOVW_UABS_G0, abs_small
        .inst   0xd2800005      // movz x5, #0
        .reloc  ., R_AARCH64_MOVW_UABS_G1, abs_small
        .inst   0xd2a00005      // movz x5, #0, lsl #16
        .reloc  ., R_AARCH64_MOVW_UABS_G2, abs_big
        .inst   0xd2c00005      // movz x5, #0, lsl #32
        .endif
        // R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21 and TLSIE_LD64_GOTTPREL_LO12_NC.
        adrp    x6, :gottprel:tls_a
        ldr     x6, [x6, :gottprel_lo12:tls_a]
        // R_AARCH64_TLSLE_*.
        movz    x7, #:tprel_g2:tls_b
        movk    x7, #:tprel_g1_nc:tls_b
        movk    x7, #:tprel_g0_nc:tls_b
        movz    x7, #:tprel_g1:tls_b
        movz    x7, #:tprel_g0:tls_b
        add     x7, x7, #:tprel_hi12:tls_b, lsl #12
        add     x7, x7, #:tprel_lo12_nc:tls_b
        ldrb    w8, [x7, #:tprel_lo12_nc:tls_b]
        ldrh    w8, [x7, #:tprel_lo12_nc:tls_b]
        ldr     w8, [x7, #:tprel_lo12_nc:tls_b]
        ldr     x8, [x7, #:tprel_lo12_nc:tls_b]
        .ifndef GNU_LD
        ldr     q8, [x7, #:tprel_lo12_nc:tls_q]
        .endif
        // R_AARCH64_TLSDESC_*: the descriptor sequence.
        adrp    x0, :tlsdesc:tls_c
        ldr     x1, [x0, :tlsdesc_lo12:tls_c]
        add     x0, x0, :tlsdesc_lo12:tls_c
        .tlsdesccall tls_c
        blr     x1
        // R_AARCH64_CALL26 to an undefined weak function, guarded as crtbeginS.o guards it.
        bl      weak_undef
        b       func_b

        .globl  func_a
        .type   func_a, %function
func_a:
        ret
        .globl  func_b
        .type   func_b, %function
func_b:
        ret

        .data
        .balign 16
        .globl  quad_a
quad_a:
        .zero   16
        .globl  dword_a
dword_a:
        .xword  0
        .globl  word_a
word_a:
        .word   0
        .globl  half_a
half_a:
        .hword  0
        .globl  byte_a
byte_a:
        .byte   0
        .balign 8
        .globl  data_a
data_a:
        .xword  0
        .globl  data_b
data_b:
        .xword  0
        // R_AARCH64_ABS64: a relative fixup, an absolute value, an undefined weak symbol (0).
        .xword  func_a
        .xword  data_b+16
        .reloc  ., R_AARCH64_ABS64, abs_big
        .xword  0
        .xword  weak_undef
        // R_AARCH64_PREL64, PREL32 and ABS32.
        .xword  func_a - .
        .word   func_b - .
        .reloc  ., R_AARCH64_ABS32, abs_small
        .word   0

        .section .data.rel.ro, "aw"
        .globl  relro_a
relro_a:
        .xword  func_b

        .section .tdata, "awT", %progbits
        .balign 16
        .globl  tls_q
tls_q:
        .zero   16
        .globl  tls_a
tls_a:
        .xword  0x1111
        .globl  tls_b
tls_b:
        .xword  0x2222

        .section .tbss, "awT", %nobits
        .globl  tls_c
tls_c:
        .zero   24

        .section .note.GNU-stack, "", %progbits
