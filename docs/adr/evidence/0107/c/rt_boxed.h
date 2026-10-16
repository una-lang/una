/* Today's layout, emulated (ADR 0107 evidence): every value one word; heap values are
   handles into a slot table; a record is a vector of words; ints are tagged; every
   access is an out-of-line call. */
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#define NOINLINE __attribute__((noinline))
typedef int64_t w;
typedef struct { int64_t count; int64_t tag; w *cells; int64_t len, cap; } obj;
static obj *slots; static int64_t nslots, capslots; static int64_t *freelist; static int64_t nfree;
static inline w tag_int(int64_t v) { return (v << 1) | 1; }
static inline int64_t untag(w x) { return x >> 1; }
NOINLINE w rt_data(int64_t tag) {
  int64_t i;
  if (nfree) i = freelist[--nfree];
  else { if (nslots == capslots) { capslots = capslots ? capslots * 2 : 1024; slots = realloc(slots, capslots * sizeof(obj)); freelist = realloc(freelist, capslots * sizeof(int64_t)); } i = nslots++; }
  slots[i] = (obj){1, tag, NULL, 0, 0};
  return (i + 1) << 1;
}
NOINLINE void rt_push(w o, w v) {
  obj *p = &slots[(o >> 1) - 1];
  if (p->len == p->cap) { p->cap = p->cap ? p->cap * 2 : 2; p->cells = realloc(p->cells, p->cap * sizeof(w)); }
  p->cells[p->len++] = v;
}
NOINLINE w rt_field(w o, int64_t i) {
  obj *p = &slots[(o >> 1) - 1];
  if (i < 0 || i >= p->len) abort();
  return p->cells[i];
}
NOINLINE void rt_release(w o) {
  if (!(o & 1) && o) { int64_t i = (o >> 1) - 1; if (--slots[i].count == 0) { free(slots[i].cells); freelist[nfree++] = i; } }
}
/* A fixed-width method: today it converts to a big integer and back; emulated as an
   out-of-line call on the tagged word, which understates today's cost. */
NOINLINE w rt_fixed_wrapping_add_i64(w a, w b) { return tag_int(untag(a) + untag(b)); }
