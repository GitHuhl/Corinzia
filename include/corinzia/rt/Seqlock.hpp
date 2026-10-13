#pragma once
// Single-writer seqlock: the audio thread publishes a small POD every block,
// any number of readers (UI render thread, QML timer) read a consistent copy
// without ever blocking the writer. Readers retry if they raced a write.

#include <atomic>
#include <cstdint>
#include <cstring>
#include <type_traits>

namespace corinzia::rt {

template <class T>
class Seqlock {
    static_assert(std::is_trivially_copyable_v<T>);

public:
    void store(const T& v) noexcept {               // writer: audio thread only
        const auto s = seq_.load(std::memory_order_relaxed);
        seq_.store(s + 1, std::memory_order_relaxed);
        std::atomic_thread_fence(std::memory_order_release);
        copyIn(v);
        std::atomic_thread_fence(std::memory_order_release);
        seq_.store(s + 2, std::memory_order_relaxed);
    }

    T load() const noexcept {                       // readers: any thread
        T out;
        for (;;) {
            const auto s1 = seq_.load(std::memory_order_acquire);
            if (s1 & 1) continue;                   // write in progress
            copyOut(out);
            std::atomic_thread_fence(std::memory_order_acquire);
            if (seq_.load(std::memory_order_relaxed) == s1) return out;
        }
    }

private:
    // Word-wise relaxed atomics keep the data race formally defined.
    static constexpr std::size_t kWords = (sizeof(T) + 7) / 8;
    void copyIn(const T& v) noexcept {
        std::uint64_t tmp[kWords] = {};
        std::memcpy(tmp, &v, sizeof(T));
        for (std::size_t i = 0; i < kWords; ++i) data_[i].store(tmp[i], std::memory_order_relaxed);
    }
    void copyOut(T& v) const noexcept {
        std::uint64_t tmp[kWords];
        for (std::size_t i = 0; i < kWords; ++i) tmp[i] = data_[i].load(std::memory_order_relaxed);
        std::memcpy(&v, tmp, sizeof(T));
    }

    std::atomic<std::uint64_t> seq_{0};
    std::atomic<std::uint64_t> data_[kWords] = {};
};

}  // namespace corinzia::rt
