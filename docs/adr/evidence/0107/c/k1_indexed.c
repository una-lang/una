/* The proposed layout (ADR 0107 rule 2): handles stay slot indices into a segmented slot
   table whose segments never move: segment k holds 2^(k+4) slots, and a fixed directory
   of segment addresses is reached by the index's leading bit. A record is one object, a
   header and its fields inline (I64 raw). A field read is inline: a checked index, the
   segment and slot, a checked layout id and generation, then a load. A handle is
   (generation << 33) | ((index + 1) << 1). A failed check takes the slow path (here:
   abort), so a stale or corrupt handle never reads another object. */
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#define NOINLINE __attribute__((noinline))
#define BASE 4
typedef struct { int64_t count; int64_t layout; } header;
typedef struct { header h; int64_t x, y; } Point;
typedef struct { int64_t count; int64_t layout; int64_t generation; header *addr; } slot;
static slot *dir[60]; static uint64_t nslots; static int64_t *freelist; static uint64_t nfree, capfree;
static inline slot *at(uint64_t i) {
  uint64_t j = i + ((uint64_t)1 << BASE);
  int k = 63 - __builtin_clzll(j);
  return &dir[k - BASE][j - ((uint64_t)1 << k)];
}
NOINLINE int64_t rt_alloc(int64_t size, int64_t layout) {
  uint64_t i;
  if (nfree) i = (uint64_t)freelist[--nfree];
  else {
    i = nslots++;
    uint64_t j = i + ((uint64_t)1 << BASE); int k = 63 - __builtin_clzll(j);
    if (!dir[k - BASE]) dir[k - BASE] = calloc((size_t)1 << k, sizeof(slot));
    at(i)->generation = 1;
    if (nslots > capfree) { capfree = capfree ? capfree * 2 : 1024; freelist = realloc(freelist, capfree * sizeof(int64_t)); }
  }
  header *h = malloc((size_t)size); h->count = 1; h->layout = layout;
  slot *s = at(i); s->count = 1; s->layout = layout; s->addr = h;
  return (int64_t)(((uint64_t)s->generation << 33) | ((i + 1) << 1));
}
NOINLINE void rt_free(int64_t o) { uint64_t i = (((uint64_t)o >> 1) & 0xffffffffu) - 1; slot *s = at(i); free(s->addr); s->count = 0; s->layout = 0; s->addr = NULL; if (++s->generation < (1 << 30) - 1) freelist[nfree++] = (int64_t)i; }
static inline slot *check(int64_t o, int64_t layout) {
  if (o & 1) abort();
  uint64_t i = (((uint64_t)o >> 1) & 0xffffffffu) - 1;
  if (i >= nslots) abort();
  slot *s = at(i);
  if (s->layout != layout || s->generation != (int64_t)((uint64_t)o >> 33)) abort();
  return s;
}
static inline void release(int64_t o) { slot *s = check(o, 7); if (--s->count == 0) rt_free(o); }
int main(void) {
  uint64_t total = 0, k = 0;
  for (int64_t i = 0; i < 5000000; i++) {
    int64_t p = rt_alloc(sizeof(Point), 7);
    Point *q = (Point *)check(p, 7)->addr; q->x = (int64_t)k; q->y = 3;
    total += (uint64_t)((Point *)check(p, 7)->addr)->x;
    total += (uint64_t)((Point *)check(p, 7)->addr)->y;
    release(p);
    k += 1;
  }
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
