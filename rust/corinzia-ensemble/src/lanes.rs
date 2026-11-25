//! Several values worked on side by side: one per player of a section, so a
//! section's players are computed together (the compiler turns these
//! fixed-size arrays' element-wise arithmetic into vector instructions; no
//! platform intrinsics). `Fx<1>` is a single player: the same code.

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub};

/// `N` single-precision values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fx<const N: usize>(pub [f32; N]);

/// `N` double-precision values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dx<const N: usize>(pub [f64; N]);

macro_rules! lanes {
    ($t:ident, $e:ty) => {
        impl<const N: usize> Default for $t<N> {
            fn default() -> Self {
                $t([0.0; N])
            }
        }
        impl<const N: usize> From<$e> for $t<N> {
            fn from(x: $e) -> Self {
                $t([x; N])
            }
        }
        impl<const N: usize> From<[$e; N]> for $t<N> {
            fn from(x: [$e; N]) -> Self {
                $t(x)
            }
        }
        impl<const N: usize> $t<N> {
            pub const fn splat(x: $e) -> Self {
                $t([x; N])
            }
            #[inline(always)]
            pub fn map(self, f: impl Fn($e) -> $e) -> Self {
                $t(std::array::from_fn(|i| f(self.0[i])))
            }
            #[inline(always)]
            pub fn zip(self, o: Self, f: impl Fn($e, $e) -> $e) -> Self {
                $t(std::array::from_fn(|i| f(self.0[i], o.0[i])))
            }
            #[inline(always)]
            pub fn max(self, o: Self) -> Self {
                self.zip(o, <$e>::max)
            }
            #[inline(always)]
            pub fn min(self, o: Self) -> Self {
                self.zip(o, <$e>::min)
            }
            #[inline(always)]
            pub fn clamp(self, lo: $e, hi: $e) -> Self {
                self.map(|x| x.clamp(lo, hi))
            }
            #[inline(always)]
            pub fn abs(self) -> Self {
                self.map(<$e>::abs)
            }
            /// `a` where `mask`, else `b`.
            #[inline(always)]
            pub fn select(mask: [bool; N], a: Self, b: Self) -> Self {
                $t(std::array::from_fn(|i| if mask[i] { a.0[i] } else { b.0[i] }))
            }
            /// All the values added.
            pub fn sum(self) -> $e {
                self.0.iter().sum()
            }
        }
        impl<const N: usize> Add for $t<N> {
            type Output = Self;
            #[inline(always)]
            fn add(self, o: Self) -> Self {
                self.zip(o, |a, b| a + b)
            }
        }
        impl<const N: usize> AddAssign for $t<N> {
            #[inline(always)]
            fn add_assign(&mut self, o: Self) {
                *self = *self + o;
            }
        }
        impl<const N: usize> Sub for $t<N> {
            type Output = Self;
            #[inline(always)]
            fn sub(self, o: Self) -> Self {
                self.zip(o, |a, b| a - b)
            }
        }
        impl<const N: usize> Mul for $t<N> {
            type Output = Self;
            #[inline(always)]
            fn mul(self, o: Self) -> Self {
                self.zip(o, |a, b| a * b)
            }
        }
        impl<const N: usize> Div for $t<N> {
            type Output = Self;
            #[inline(always)]
            fn div(self, o: Self) -> Self {
                self.zip(o, |a, b| a / b)
            }
        }
        impl<const N: usize> Add<$e> for $t<N> {
            type Output = Self;
            #[inline(always)]
            fn add(self, o: $e) -> Self {
                self.map(|a| a + o)
            }
        }
        impl<const N: usize> Sub<$e> for $t<N> {
            type Output = Self;
            #[inline(always)]
            fn sub(self, o: $e) -> Self {
                self.map(|a| a - o)
            }
        }
        impl<const N: usize> Mul<$e> for $t<N> {
            type Output = Self;
            #[inline(always)]
            fn mul(self, o: $e) -> Self {
                self.map(|a| a * o)
            }
        }
        impl<const N: usize> Div<$e> for $t<N> {
            type Output = Self;
            #[inline(always)]
            fn div(self, o: $e) -> Self {
                self.map(|a| a / o)
            }
        }
        impl<const N: usize> Neg for $t<N> {
            type Output = Self;
            #[inline(always)]
            fn neg(self) -> Self {
                self.map(|a| -a)
            }
        }
    };
}

lanes!(Fx, f32);
lanes!(Dx, f64);

impl<const N: usize> Fx<N> {
    /// As double precision.
    #[inline(always)]
    pub fn wide(self) -> Dx<N> {
        Dx(std::array::from_fn(|i| self.0[i] as f64))
    }
}

impl<const N: usize> Dx<N> {
    /// As single precision.
    #[inline(always)]
    pub fn narrow(self) -> Fx<N> {
        Fx(std::array::from_fn(|i| self.0[i] as f32))
    }
}
