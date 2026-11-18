/* The absolute symbols of the oracle's fixtures (x86_64.s and aarch64.s), in an object of their
   own so that the assemblers of the fixtures keep the relocations against them. Hidden, as the
   executable's own symbols are: GNU ld refuses a narrow absolute relocation against a global
   symbol of default visibility in a position-independent executable. */
        .globl  abs_small
        .hidden abs_small
        .set    abs_small, 0x1234
        .globl  abs_neg
        .hidden abs_neg
        .set    abs_neg, -8
        .globl  abs_big
        .hidden abs_big
        .set    abs_big, 0x123456789abc

        .section .note.GNU-stack, "", %progbits
