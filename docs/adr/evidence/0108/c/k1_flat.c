/* ADR 0108 k1, flat: Point(x: I64, y: I64) is two cells in registers; no allocation.
   The C compiler may reduce the loop entirely, as it would for Rust's struct. */
#include <stdint.h>
#include <stdio.h>
typedef struct { int64_t x, y; } Point;
int main(void) {
  uint64_t total = 0, k = 0;
  for (int64_t i = 0; i < 5000000; i++) {
    Point p = { (int64_t)k, 3 };
    total += (uint64_t)p.x; total += (uint64_t)p.y;
    k += 1;
  }
  printf("%lld\n", (long long)(int64_t)total);
  return 0;
}
