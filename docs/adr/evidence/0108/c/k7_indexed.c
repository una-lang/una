/* ADR 0108 k7 under ADR 0107: Polygon(a, b, c: Point) is an object of three handles, each
   Point its own object: 4 allocations per construction. The loop builds one polygon per
   1,000 iterations, and reads poly.a each iteration and passes it to generic code. */
#include "rt_indexed.h"
#define POINT 7
#define POLY 11
NOINLINE int64_t generic(int64_t word) { int64_t *c = check(word, POINT)->addr; int64_t v = c[0] + c[1]; release(word, POINT); return v; }
static int64_t point(int64_t x, int64_t y) { int64_t p = rt_alloc(2, POINT); int64_t *c = check(p, POINT)->addr; c[0] = x; c[1] = y; return p; }
static void free_poly(int64_t poly) {
  slot *s = check(poly, POLY);
  if (--s->count == 0) { for (int j = 0; j < 3; j++) release(s->addr[j], POINT); rt_free(poly); }
}
int main(void) {
  uint64_t total = 0; int64_t poly = 0;
  for (int64_t i = 0; i < 5000000; i++) {
    if (i % 1000 == 0) {
      if (poly) free_poly(poly);
      poly = rt_alloc(3, POLY); int64_t a = point(i, 1), b = point(2, 3), c = point(4, 5);
      int64_t *cells = check(poly, POLY)->addr; cells[0] = a; cells[1] = b; cells[2] = c;
    }
    int64_t a = check(poly, POLY)->addr[0]; retain(a, POINT); total += (uint64_t)generic(a);
  }
  free_poly(poly);
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
