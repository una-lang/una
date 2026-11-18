#include "rt_boxed.h"
NOINLINE w rt_fixed_u8_wrapping_add(w a, w b) { return tag_int((untag(a) + untag(b)) & 255); }
/* U64 values above 2^62 are big-integer handles today; emulated as raw u64 in an
   out-of-line call, which again understates the cost. */
NOINLINE uint64_t rt_fixed_u64_mix(uint64_t h, w x) { return (h ^ (uint64_t)untag(x)) * 1099511628211ULL; }
int main(void) {
  w bytes = rt_data(0); w b = tag_int(0);
  for (int64_t i = 0; i < 2000000; i++) { rt_push(bytes, b); b = rt_fixed_u8_wrapping_add(b, tag_int(7)); }
  uint64_t h = 14695981039346656037ULL;
  int64_t n = slots[(bytes >> 1) - 1].len;
  for (int64_t i = 0; i < n; i++) h = rt_fixed_u64_mix(h, rt_field(bytes, i));
  printf("%llu\n", (unsigned long long)h);
  return 0;
}
