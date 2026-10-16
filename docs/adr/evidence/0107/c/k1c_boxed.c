#include "rt_boxed.h"
/* k1c: as k1, with a runtime call between the two field reads. */
static volatile int64_t polls;
NOINLINE void rt_poll(void) { polls++; }
int main(void) {
  w total = tag_int(0), k = tag_int(0);
  for (int64_t i = 0; i < 5000000; i++) {
    w p = rt_data(0); rt_push(p, k); rt_push(p, tag_int(3));
    total = rt_fixed_wrapping_add_i64(total, rt_field(p, 0));
    rt_poll();
    total = rt_fixed_wrapping_add_i64(total, rt_field(p, 1));
    rt_release(p);
    k = rt_fixed_wrapping_add_i64(k, tag_int(1));
  }
  printf("%lld\n", (long long)untag(total));
  return 0;
}
