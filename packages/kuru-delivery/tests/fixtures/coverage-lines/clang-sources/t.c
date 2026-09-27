#include <stdio.h>
#define TWICE(x) ((x) + (x))
#define PICK(c, a, b) ((c) ? (a) : (b))
#define CHECK(v) do { if ((v) < 0) { puts("neg"); } } while (0)

static int helper(int v) {
  if (v > 3)
    return TWICE(v);
  else
    return PICK(v, 1,
                2);
}

#if 0
int dead(void) { return 1; }
#endif

int loop(int n) {
  int s = 0;
  for (int i = 0; i < n; i++) {
    if (i % 2) continue;
    s += helper(i);
  }
  CHECK(s);
  switch (n) {
  case 1: return 1;
  case 2:
    return 2;
  default: break;
  }
  return s;
}

int unused(int a) { return a ? 1 :
  0; }

int main(int argc, char **argv) {
  (void)argv;
  int r = loop(argc + 4);
  if (r > 100) { r = 0; } else
  { r = 1; }
  return r - 1;
}
