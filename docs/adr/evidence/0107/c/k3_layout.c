/* The proposed layout: a loop's state lives in registers; Int stays tagged with a checked
   operation (ADR 0106 keeps it). */
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
static inline int64_t tag_int(int64_t v) { return (v << 1) | 1; }
static inline int64_t untag(int64_t x) { return x >> 1; }
int main(void) {
  int64_t a = tag_int(0), c = tag_int(1);
  for (int64_t i = 0; i < 10000000; i++) {
    int64_t a2 = c, r;
    if (__builtin_sub_overflow(untag(tag_int(i)), untag(a2), &r)) abort();
    a = a2; c = tag_int(r);
  }
  printf("%lld %lld\n", (long long)untag(a), (long long)untag(c));
  return 0;
}
