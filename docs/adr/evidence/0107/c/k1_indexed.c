/* The proposed layout with handles kept as slot-table indices: a record is one object, a
   header and its fields inline (I64 raw); the slot holds the object's address and kind.
   A field read is inline: a checked slot index, a checked kind, then a load. A failed
   check takes the slow path (here: abort), so a stale or corrupt handle never reads
   outside an object of the expected kind. */
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#define NOINLINE __attribute__((noinline))
typedef struct { int64_t count; int64_t kind; } header;
typedef struct { header h; int64_t x, y; } Point;
typedef struct { header *addr; int64_t kind; } slot;
static slot *slots; static int64_t nslots, capslots; static int64_t *freelist; static int64_t nfree;
NOINLINE int64_t rt_alloc(int64_t size, int64_t kind) {
  int64_t i;
  if (nfree) i = freelist[--nfree];
  else { if (nslots == capslots) { capslots = capslots ? capslots * 2 : 1024; slots = realloc(slots, capslots * sizeof(slot)); freelist = realloc(freelist, capslots * sizeof(int64_t)); } i = nslots++; }
  header *h = malloc((size_t)size); h->count = 1; h->kind = kind;
  slots[i] = (slot){h, kind};
  return (i + 1) << 1;
}
NOINLINE void rt_free(int64_t o) { int64_t i = (o >> 1) - 1; free(slots[i].addr); slots[i] = (slot){NULL, 0}; freelist[nfree++] = i; }
static inline header *deref(int64_t o, int64_t kind) {
  uint64_t i = (uint64_t)((o >> 1) - 1);
  if (i >= (uint64_t)nslots || slots[i].kind != kind) abort();
  return slots[i].addr;
}
static inline void release(int64_t o) { header *h = deref(o, 7); if (--h->count == 0) rt_free(o); }
int main(void) {
  uint64_t total = 0, k = 0;
  for (int64_t i = 0; i < 5000000; i++) {
    int64_t p = rt_alloc(sizeof(Point), 7);
    Point *q = (Point *)deref(p, 7); q->x = (int64_t)k; q->y = 3;
    total += (uint64_t)((Point *)deref(p, 7))->x;
    total += (uint64_t)((Point *)deref(p, 7))->y;
    release(p);
    k += 1;
  }
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
