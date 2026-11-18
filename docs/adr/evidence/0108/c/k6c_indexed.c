/* ADR 0108 k6c under ADR 0107: one Point object; g(p) is called 5,000,000 times with the
   same p, a shared use, and g passes it to generic code. No allocation in the loop. */
#include "rt_indexed.h"
#define POINT 7
NOINLINE int64_t generic(int64_t word) { int64_t *c = check(word, POINT)->addr; int64_t v = c[0] + c[1]; release(word, POINT); return v; }
NOINLINE int64_t g(int64_t p) { return generic(p); }
int main(void) {
  int64_t p = rt_alloc(2, POINT); int64_t *c = check(p, POINT)->addr; c[0] = 4; c[1] = 3;
  uint64_t total = 0;
  for (int64_t i = 0; i < 5000000; i++) { retain(p, POINT); total += (uint64_t)g(p); }
  release(p, POINT);
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
