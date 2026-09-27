#pragma once
#define SQUARE(x) ((x) * (x))
template <typename T> T clampit(T v, T lo, T hi) {
  if (v < lo)
    return lo;
  if (v > hi) return hi;
  return v;
}
inline int header_fn(int v) {
#if 0
  return -1;
#endif
  return SQUARE(v);
}
