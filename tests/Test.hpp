#pragma once
// Minimal self-registering test harness (no external dependency).
#include <cmath>
#include <cstdio>
#include <functional>
#include <string>
#include <vector>

namespace t {
struct Case { const char* name; std::function<void()> fn; };
inline std::vector<Case>& registry() { static std::vector<Case> r; return r; }
inline int& failures() { static int f = 0; return f; }
struct Reg { Reg(const char* n, std::function<void()> f) { registry().push_back({n, std::move(f)}); } };
}  // namespace t

#define TCAT2(a, b) a##b
#define TCAT(a, b) TCAT2(a, b)
#define TEST(name)                                                    \
    static void TCAT(test_, name)();                                  \
    static t::Reg TCAT(reg_, name)(#name, &TCAT(test_, name));        \
    static void TCAT(test_, name)()

#define CHECK(cond)                                                                       \
    do {                                                                                  \
        if (!(cond)) {                                                                    \
            std::fprintf(stderr, "  FAIL %s:%d: %s\n", __FILE__, __LINE__, #cond);        \
            ++t::failures();                                                              \
        }                                                                                 \
    } while (0)
#define CHECK_NEAR(a, b, eps) CHECK(std::abs(double(a) - double(b)) <= (eps))
