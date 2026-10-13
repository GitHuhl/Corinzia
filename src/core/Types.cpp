#include "corinzia/core/Types.hpp"

namespace corinzia {

std::string ElementId::toSvgId() const {
    static constexpr char kDigits[] = "0123456789abcdefghijklmnopqrstuvwxyz";
    char buf[16];
    int n = 0;
    std::uint64_t v = value;
    do { buf[n++] = kDigits[v % 36]; v /= 36; } while (v != 0);
    std::string out = "e-";
    while (n > 0) out.push_back(buf[--n]);
    return out;
}

ElementId ElementId::fromSvgId(std::string_view s) noexcept {
    if (s.size() < 3 || s.size() > 15 || s[0] != 'e' || s[1] != '-') return {};
    std::uint64_t v = 0;
    for (char c : s.substr(2)) {
        int d;
        if (c >= '0' && c <= '9') d = c - '0';
        else if (c >= 'a' && c <= 'z') d = c - 'a' + 10;
        else return {};
        v = v * 36 + std::uint64_t(d);
    }
    return ElementId{v};
}

}  // namespace corinzia
