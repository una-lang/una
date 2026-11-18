/* ADR 0108 k6, flat: the Point is unboxed; its first boxing allocates the companion, and
   the second and third reuse it with a retain. */
#include "rt_indexed.h"
#define POINT 7
NOINLINE int64_t generic(int64_t word) {
  int64_t *c = check(word, POINT)->addr; int64_t v = c[0] + c[1];
  release(word, POINT); return v;
}
int main(void) {
  uint64_t total = 0;
  for (int64_t i = 0; i < 5000000; i++) {
    int64_t x = i, y = 3, companion = 0;
    for (int j = 0; j < 3; j++) {
      if (!companion) { companion = rt_alloc(2, POINT); int64_t *c = check(companion, POINT)->addr; c[0] = x; c[1] = y; }
      retain(companion, POINT); total += (uint64_t)generic(companion);
    }
    release(companion, POINT);
  }
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
