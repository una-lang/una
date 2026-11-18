#include "rt_boxed.h"
NOINLINE w rt_int_sub(w a, w b) { int64_t r; if (__builtin_sub_overflow(untag(a), untag(b), &r)) abort(); return tag_int(r); }
int main(void) {
  w a = tag_int(0), c = tag_int(1);
  for (int64_t i = 0; i < 10000000; i++) {
    /* The body returns the new state as a heap tuple, which the loop unpacks. */
    w a2 = c; w c2 = rt_int_sub(tag_int(i), a2);
    w t = rt_data(0); rt_push(t, a2); rt_push(t, c2);
    a = rt_field(t, 0); c = rt_field(t, 1); rt_release(t);
  }
  printf("%lld %lld\n", (long long)untag(a), (long long)untag(c));
  return 0;
}
