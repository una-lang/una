/* ADR 0108 k6c, flat: p is unboxed; g takes its cells and its companion. The first boxing
   in g allocates the companion and returns it to the caller, so later calls reuse it. */
#include "rt_indexed.h"
#define POINT 7
NOINLINE int64_t generic(int64_t word) { int64_t *c = check(word, POINT)->addr; int64_t v = c[0] + c[1]; release(word, POINT); return v; }
typedef struct { int64_t v; int64_t companion; } result;
NOINLINE result g(int64_t x, int64_t y, int64_t companion) {
  if (!companion) { companion = rt_alloc(2, POINT); int64_t *c = check(companion, POINT)->addr; c[0] = x; c[1] = y; }
  retain(companion, POINT);
  return (result){ generic(companion), companion };
}
int main(void) {
  int64_t x = 4, y = 3, companion = 0;
  uint64_t total = 0;
  for (int64_t i = 0; i < 5000000; i++) { result r = g(x, y, companion); companion = r.companion; total += (uint64_t)r.v; }
  if (companion) release(companion, POINT);
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
