/* ADR 0107's proposed runtime, as in its evidence kit (k1_indexed.c): handles are
   (generation << 33) | ((index + 1) << 1) into a segmented slot table that never moves;
   an access checks bit 0, the index, the layout and the generation. */
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#define NOINLINE __attribute__((noinline))
#define BASE 4
typedef struct { int64_t count; int64_t layout; int64_t generation; int64_t *addr; } slot;
static slot *dir[60]; static uint64_t nslots; static int64_t *freelist; static uint64_t nfree, capfree;
static inline slot *at(uint64_t i) {
  uint64_t j = i + ((uint64_t)1 << BASE);
  int k = 63 - __builtin_clzll(j);
  return &dir[k - BASE][j - ((uint64_t)1 << k)];
}
NOINLINE int64_t rt_alloc(int64_t cells, int64_t layout) {
  uint64_t i;
  if (nfree) i = (uint64_t)freelist[--nfree];
  else {
    i = nslots++;
    uint64_t j = i + ((uint64_t)1 << BASE); int k = 63 - __builtin_clzll(j);
    if (!dir[k - BASE]) dir[k - BASE] = calloc((size_t)1 << k, sizeof(slot));
    at(i)->generation = 1;
    if (nslots > capfree) { capfree = capfree ? capfree * 2 : 1024; freelist = realloc(freelist, capfree * sizeof(int64_t)); }
  }
  slot *s = at(i); s->count = 1; s->layout = layout; s->addr = malloc((size_t)cells * 8);
  return (int64_t)(((uint64_t)s->generation << 33) | ((i + 1) << 1));
}
static inline slot *check(int64_t o, int64_t layout) {
  if (o & 1) abort();
  uint64_t i = (((uint64_t)o >> 1) & 0xffffffffu) - 1;
  if (i >= nslots) abort();
  slot *s = at(i);
  if (s->layout != layout || s->generation != (int64_t)((uint64_t)o >> 33)) abort();
  return s;
}
NOINLINE void rt_free(int64_t o) { uint64_t i = (((uint64_t)o >> 1) & 0xffffffffu) - 1; slot *s = at(i); free(s->addr); s->count = 0; s->layout = 0; s->addr = NULL; if (++s->generation < (1 << 30) - 1) freelist[nfree++] = (int64_t)i; }
static inline void retain(int64_t o, int64_t layout) { check(o, layout)->count++; }
static inline void release(int64_t o, int64_t layout) { slot *s = check(o, layout); if (--s->count == 0) rt_free(o); }
