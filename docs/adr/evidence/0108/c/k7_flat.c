/* ADR 0108 k7, flat: Polygon(a, b, c: Point) is one object of six raw cells (6 cells > 4,
   so not flat itself): 1 allocation per construction. poly.a read each iteration is cells
   with no companion, so passing it to generic code allocates each iteration. */
#include "rt_indexed.h"
#define POINT 7
#define POLY 11
NOINLINE int64_t generic(int64_t word) { int64_t *c = check(word, POINT)->addr; int64_t v = c[0] + c[1]; release(word, POINT); return v; }
int main(void) {
  uint64_t total = 0; int64_t poly = 0;
  for (int64_t i = 0; i < 5000000; i++) {
    if (i % 1000 == 0) {
      if (poly) release(poly, POLY);
      poly = rt_alloc(6, POLY); int64_t *cells = check(poly, POLY)->addr;
      cells[0] = i; cells[1] = 1; cells[2] = 2; cells[3] = 3; cells[4] = 4; cells[5] = 5;
    }
    int64_t *cells = check(poly, POLY)->addr; int64_t x = cells[0], y = cells[1];
    int64_t box = rt_alloc(2, POINT); int64_t *c = check(box, POINT)->addr; c[0] = x; c[1] = y;
    total += (uint64_t)generic(box);
  }
  release(poly, POLY);
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
