/* ADR 0108 k6 under ADR 0107: each iteration builds a Point (one object) and passes it to
   an out-of-line generic function three times, each a shared use with a retain. */
#include "rt_indexed.h"
#define POINT 7
NOINLINE int64_t generic(int64_t word) {
  int64_t *c = check(word, POINT)->addr; int64_t v = c[0] + c[1];
  release(word, POINT); return v;
}
int main(void) {
  uint64_t total = 0;
  for (int64_t i = 0; i < 5000000; i++) {
    int64_t p = rt_alloc(2, POINT); int64_t *c = check(p, POINT)->addr; c[0] = i; c[1] = 3;
    for (int j = 0; j < 3; j++) { retain(p, POINT); total += (uint64_t)generic(p); }
    release(p, POINT);
  }
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
