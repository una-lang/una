/* ADR 0108 k5, flat: Pair(a: Text, b: Text, n: I64) is three cells; a shared use passes
   the cells and retains both word cells, and the callee releases both. */
#include "rt_indexed.h"
#define TEXT 3
NOINLINE int64_t use(int64_t a, int64_t b, int64_t n) {
  release(a, TEXT); release(b, TEXT); return n;
}
int main(void) {
  int64_t a = rt_alloc(1, TEXT), b = rt_alloc(1, TEXT), n = 7;
  uint64_t total = 0;
  for (int64_t i = 0; i < 5000000; i++) { retain(a, TEXT); retain(b, TEXT); total += (uint64_t)use(a, b, n); }
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
