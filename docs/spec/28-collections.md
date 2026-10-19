# 28. Collections

Status: Normative, edition `0.4` (ADR 0119). This chapter grows with the implementation of ADR 0119: it holds the rules that have landed, and ADR 0119 holds the rest until they land.

## 1. Keyed hashing

`HashMap` and `HashSet` place their keys by a keyed hash. These rules define it for both engines, which MUST implement them with one module (`crates/una-hash`). Nothing of this section is observable to a program: no operation returns a hash or a key, and no hash or key appears in a trace, the report, the Core export or a message.

1. **The function.** The hash of a key is SipHash-2-4, under the 128-bit key of its table, of the key's hash encoding (rule 2). The result is 64 bits.
2. **The hash encoding.** The encoding of a value is by value, over the form both engines share, and is defined only here:
   - an integer of any type, a `Bool` as 0 or 1, `unit` as 0, a variant without payload as its declaration index, and an `Id` as its value, by its mathematical value: when it lies in -2^63 to 2^63 - 1, the byte 0 and 8 bytes of two's complement, little-endian; otherwise the byte 1, a sign byte (0 for positive, 1 for negative), the length of its magnitude in bytes as 8 bytes little-endian, and the magnitude, little-endian, with no zero byte at its end;
   - an `F64`: the byte 2 and the 8 bytes of its canonical bits, little-endian;
   - an `F32`: the integer whose value is its 32 canonical bits;
   - a `Text` or a `Hashed`: its length in bytes as an integer, then its UTF-8 or digest bytes;
   - an object with a layout (a record, a variant with a payload, a tuple, an `Option` or `Result` with a payload): its variant's declaration index as an integer, then each cell in order;
   - a `List`, a `Stack`, a `Queue`, a `Set` or a `Map`: its length as an integer, then its elements, or each entry's key and value, in its listing order;
   - a `BitSet`: its size as an integer, then its 64-bit words, little-endian, with the bits past the size at zero.

   Two values equal by `==` MUST have the same encoding, and two distinct values of one type MUST have distinct encodings.
3. **The process key.** A run whose reachable code can make a hash table reads 128 bits from the operating system's cryptographic random source, before `main` and before any effect. When the host gives no random bytes, the run MUST abort with U6009, exit code 1, before any effect; no fixed or guessable key MAY replace it. A run whose code cannot make a hash table MUST NOT read it.
4. **The key of a table.** Each new table takes a 128-bit key: the two 64-bit results of SipHash-2-4, under the process key, of the table's counter value as 8 bytes little-endian, and of the same 8 bytes followed by the byte 0x01. The counter counts the tables of the run and is not observable; a table made when it has reached 2^64 - 1 aborts the run with U6003. A copy of a table, and the copy at a task's fork or join, keep its key.
5. **The table.** A table holds its entries in a dense vector in insertion order, and an index of open addressing with linear probing whose capacity is the smallest power of two at least twice the number of entries, tombstones included, and at least 8. The index is never iterated: a listing walks the entries, so no listing depends on a hash.
6. **Test builds.** A fixed process key, a counter of the index's probes and a mode in which every key hashes alike exist only in the toolchain's own test builds. No shipped artifact MAY contain them, and no environment variable, flag or `una.toml` key MAY set the key.
