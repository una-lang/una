/* The proposed layout: a List[U8] is a packed array of bytes; U8 and U64 are raw. */
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
typedef struct { int64_t count; int64_t len, cap; uint8_t *items; } u8list;
int main(void) {
  u8list l = {1, 0, 0, NULL}; uint8_t b = 0;
  for (int64_t i = 0; i < 2000000; i++) {
    if (l.len == l.cap) { l.cap = l.cap ? l.cap * 2 : 16; l.items = realloc(l.items, (size_t)l.cap); }
    l.items[l.len++] = b; b = (uint8_t)(b + 7);
  }
  uint64_t h = 14695981039346656037ULL;
  for (int64_t i = 0; i < l.len; i++) h = (h ^ (uint64_t)l.items[i]) * 1099511628211ULL;
  printf("%llu\n", (unsigned long long)h);
  free(l.items);
  return 0;
}
