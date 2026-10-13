#include "Test.hpp"

#include <cstring>

int main(int argc, char** argv) {
    int run = 0;
    for (auto& c : t::registry()) {
        if (argc > 1 && !std::strstr(c.name, argv[1])) continue;
        const int before = t::failures();
        c.fn();
        std::printf("%s %s\n", t::failures() == before ? "[ OK ]" : "[FAIL]", c.name);
        ++run;
    }
    std::printf("%d tests, %d failed checks\n", run, t::failures());
    return t::failures() == 0 ? 0 : 1;
}
