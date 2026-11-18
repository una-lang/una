/* ADR 0108 k5 under ADR 0107: Pair(a: Text, b: Text, n: I64) is one object; a shared
   use passes its handle with one retain, and the callee releases it. */
#include "rt_indexed.h"
#define TEXT 3
#define PAIR 9
NOINLINE int64_t use(int64_t pair) {
  int64_t *c = check(pair, PAIR)->addr; int64_t n = c[2];
  release(pair, PAIR); return n;
}
int main(void) {
  int64_t a = rt_alloc(1, TEXT), b = rt_alloc(1, TEXT);
  int64_t pair = rt_alloc(3, PAIR); int64_t *c = check(pair, PAIR)->addr; c[0] = a; c[1] = b; c[2] = 7;
  uint64_t total = 0;
  for (int64_t i = 0; i < 5000000; i++) { retain(pair, PAIR); total += (uint64_t)use(pair); }
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
