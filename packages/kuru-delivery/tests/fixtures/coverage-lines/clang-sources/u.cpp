#include "h.hpp"
#include <cstdio>
#define LOG(m) do { std::puts(m); } while (0)
template <typename T> struct Box {
  T v;
  T get() const { return v > 0 ? v :
      -v; }
};
int work(int n) {
  auto lam = [&](int k) {
    if (k > n) return k;
    return n;
  };
  int s = 0;
  for (int i = 0; i < n; ++i) {
#ifdef NEVER
    s += 1000;
#endif
    s += lam(i) + header_fn(i);
    if (s > 50) { LOG("big"); break; }
  }
  return clampit(s, 0, 40) + (int)clampit(1.5, 0.0, 1.0);
}
int main(int argc, char **) {
  Box<int> b{argc};
  Box<long> c{-3};
  int r = work(argc + 5) + b.get();
  if (argc > 5)
    r += (int)c.get();
  return r == 0;
}
