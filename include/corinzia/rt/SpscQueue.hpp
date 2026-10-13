#pragma once
// Bounded single-producer / single-consumer lock-free ring buffer.
// Wait-free on both ends, no allocation after construction, trivially safe to
// use from the realtime audio callback. One queue per direction per pair of
// threads (UI->Audio commands, Audio->UI telemetry, Audio->UI retired garbage).

#include <array>
#include <atomic>
#include <cstddef>
#include <optional>
#include <type_traits>

namespace corinzia::rt {

// Fixed rather than std::hardware_destructive_interference_size, whose value
// may differ between compiler flags and would make the layout ABI-unstable.
inline constexpr std::size_t kCacheLine = 64;

template <class T, std::size_t Capacity>
class SpscQueue {
    static_assert((Capacity & (Capacity - 1)) == 0, "Capacity must be a power of two");
    static_assert(std::is_trivially_copyable_v<T>, "RT messages must be trivially copyable (no hidden frees)");

public:
    // Producer side.
    bool tryPush(const T& v) noexcept {
        const std::size_t head = head_.load(std::memory_order_relaxed);
        if (head - tailCache_ == Capacity) {
            tailCache_ = tail_.load(std::memory_order_acquire);
            if (head - tailCache_ == Capacity) return false;   // full
        }
        slots_[head & (Capacity - 1)] = v;
        head_.store(head + 1, std::memory_order_release);
        return true;
    }

    // Consumer side.
    std::optional<T> tryPop() noexcept {
        const std::size_t tail = tail_.load(std::memory_order_relaxed);
        if (tail == headCache_) {
            headCache_ = head_.load(std::memory_order_acquire);
            if (tail == headCache_) return std::nullopt;       // empty
        }
        T v = slots_[tail & (Capacity - 1)];
        tail_.store(tail + 1, std::memory_order_release);
        return v;
    }

    std::size_t sizeApprox() const noexcept {
        return head_.load(std::memory_order_acquire) - tail_.load(std::memory_order_acquire);
    }
    static constexpr std::size_t capacity() noexcept { return Capacity; }

private:
    alignas(kCacheLine) std::atomic<std::size_t> head_{0};   // written by producer
    std::size_t tailCache_ = 0;                              // producer-private
    alignas(kCacheLine) std::atomic<std::size_t> tail_{0};   // written by consumer
    std::size_t headCache_ = 0;                              // consumer-private
    alignas(kCacheLine) std::array<T, Capacity> slots_{};
};

}  // namespace corinzia::rt
