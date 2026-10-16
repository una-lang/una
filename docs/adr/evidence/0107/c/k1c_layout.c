/* The proposed layout: a record is one object, a header and its fields inline in their
   own representation; I64 fields raw; a field read is a load at a constant offset. */
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
/* k1c: as k1, with a runtime call (rt_poll, as a budget check) between the two field
   reads, so the second read cannot share the first one's check or address. */
#define NOINLINE __attribute__((noinline))
static volatile int64_t polls;
NOINLINE void rt_poll(void) { polls++; }
typedef struct { int64_t count; int64_t kind; } header;
typedef struct { header h; int64_t x, y; } Point;
NOINLINE void *rt_alloc(size_t size) { return malloc(size); }
NOINLINE void rt_free(void *p) { free(p); }
static inline void release(header *h) { if (--h->count == 0) rt_free(h); }
int main(void) {
  uint64_t total = 0, k = 0;
  for (int64_t i = 0; i < 5000000; i++) {
    Point *p = rt_alloc(sizeof(Point)); p->h = (header){1, 7}; p->x = (int64_t)k; p->y = 3;
    total += (uint64_t)p->x; rt_poll(); total += (uint64_t)p->y;
    release(&p->h);
    k += 1;
  }
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
