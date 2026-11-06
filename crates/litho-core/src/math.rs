//! Малые математические примитивы (план 7.1, 7.2, 6.3 п. 4).
//!
//! Только `f64` и базовые операции. `dexp`/`dln`/`dpow` реализованы здесь, а не
//! взяты из стандартной библиотеки: системные `exp`/`log`/`powf` на Windows
//! (MinGW) и Linux могут отличаться в последнем бите, а от них зависят решения.

// Берём константы из core: это те же биты, но clippy не принимает их
// «приблизительные» литеральные копии, а нам важна платформенная точность.
pub const SQRT_2: f64 = std::f64::consts::SQRT_2;
pub const LN_2: f64 = std::f64::consts::LN_2;
const TWO_POW_54: f64 = 18_014_398_509_481_984.0;

/// Двумерный вектор в макро-координатах (L) или блоках.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub z: f64,
}

impl Vec2 {
    pub const fn new(x: f64, z: f64) -> Self {
        Self { x, z }
    }

    pub const fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.z + other.z)
    }

    pub const fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.z - other.z)
    }

    pub const fn scale(self, k: f64) -> Self {
        Self::new(self.x * k, self.z * k)
    }

    pub const fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.z * other.z
    }

    pub const fn perp(self) -> Self {
        Self::new(-self.z, self.x)
    }

    pub const fn dist2(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dz = self.z - other.z;
        dx * dx + dz * dz
    }

    pub fn len(self) -> f64 {
        self.dist2(Self::default()).sqrt()
    }

    pub fn normalized(self) -> Self {
        let len = self.len();
        if len > 0.0 {
            self.scale(1.0 / len)
        } else {
            Self::default()
        }
    }
}

pub fn clamp(x: f64, lo: f64, hi: f64) -> f64 {
    if x < lo {
        lo
    } else if x > hi {
        hi
    } else {
        x
    }
}

pub fn clamp01(x: f64) -> f64 {
    clamp(x, 0.0, 1.0)
}

pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Гладкая ступенька. Работает и при `a > b` (ступенька вниз).
pub fn smoothstep(a: f64, b: f64, x: f64) -> f64 {
    if a == b {
        return if x < a { 0.0 } else { 1.0 };
    }
    let t = clamp01((x - a) / (b - a));
    t * t * (3.0 - 2.0 * t)
}

/// Кусочно-линейная функция по таблице узлов. За пределами таблицы значение
/// равно крайнему. Узлы обязаны идти по возрастанию `x`.
pub fn piecewise(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    debug_assert_eq!(xs.len(), ys.len());
    if xs.is_empty() {
        return 0.0;
    }
    if x <= xs[0] {
        return ys[0];
    }
    let mut i = 1;
    while i < xs.len() {
        if x <= xs[i] {
            let t = (x - xs[i - 1]) / (xs[i] - xs[i - 1]);
            return lerp(ys[i - 1], ys[i], t);
        }
        i += 1;
    }
    ys[xs.len() - 1]
}

/// Детерминированная экспонента. Разложение `exp(x) = 2^k · exp(r)`,
/// `r ∈ [−ln2/2, ln2/2]`, ряд Тейлора по `r`, степень двойки собирается из бит
/// числа `f64`. Никаких вызовов системного `libm`.
pub fn dexp(x: f64) -> f64 {
    if x.is_nan() || x == f64::INFINITY {
        return x;
    }
    if x == f64::NEG_INFINITY {
        return 0.0;
    }
    let x = clamp(x, -700.0, 700.0);
    let kf = (x / LN_2).round();
    let k = kf as i64;
    let r = x - kf * LN_2;
    let mut sum = 1.0;
    let mut term = 1.0;
    for n in 1..=20 {
        term *= r / n as f64;
        sum += term;
    }
    let biased = k + 1023;
    if biased <= 0 || biased >= 2047 {
        return 0.0;
    }
    f64::from_bits((biased as u64) << 52) * sum
}

/// Детерминированный натуральный логарифм: `ln(m·2^e) = e·ln2 + ln(m)`,
/// `m ∈ [1, 2)`; `ln(m)` считается рядом `2·atanh((m−1)/(m+1))`.
pub fn dln(x: f64) -> f64 {
    if x <= 0.0 || !x.is_finite() {
        return f64::NAN;
    }
    let mut m = x;
    let mut shift = 0.0;
    while m < f64::MIN_POSITIVE {
        m *= TWO_POW_54;
        shift -= 54.0;
    }
    let bits = m.to_bits();
    let exponent = (((bits >> 52) & 0x7FF) as i64 - 1023) as f64;
    let mantissa = f64::from_bits((bits & 0x000F_FFFF_FFFF_FFFF) | (1023_u64 << 52));
    let y = (mantissa - 1.0) / (mantissa + 1.0);
    let y2 = y * y;
    let mut term = y;
    let mut sum = 0.0;
    for k in 0..31 {
        sum += term / (2 * k + 1) as f64;
        term *= y2;
    }
    (exponent + shift) * LN_2 + 2.0 * sum
}

/// Детерминированная степень. Используется для нецелых показателей вроде
/// `(φ/90)^1.6`. Целые степени в формулах записываются явно.
pub fn dpow(base: f64, exponent: f64) -> f64 {
    if exponent == 0.0 || base == 1.0 {
        return 1.0;
    }
    if base == 0.0 {
        return if exponent > 0.0 { 0.0 } else { f64::INFINITY };
    }
    dexp(exponent * dln(base))
}

#[cfg(test)]
mod tests {
    use super::{LN_2, Vec2, clamp, dexp, dln, dpow, lerp, piecewise, smoothstep};

    #[test]
    fn smoothstep_edges_and_reverse_direction() {
        assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
        assert!((smoothstep(0.0, 1.0, 0.5) - 0.5).abs() < 1e-15);
        assert_eq!(smoothstep(1.0, 0.0, 2.0), 0.0);
        assert_eq!(smoothstep(1.0, 0.0, -1.0), 1.0);
        assert_eq!(smoothstep(0.5, 0.5, 0.4), 0.0);
    }

    #[test]
    fn piecewise_hits_knots_exactly() {
        let xs = [0.0, 1.0, 2.0];
        let ys = [10.0, 20.0, 40.0];
        assert_eq!(piecewise(&xs, &ys, -1.0), 10.0);
        assert_eq!(piecewise(&xs, &ys, 1.0), 20.0);
        assert_eq!(piecewise(&xs, &ys, 1.5), 30.0);
        assert_eq!(piecewise(&xs, &ys, 9.0), 40.0);
        assert_eq!(piecewise(&[], &[], 3.0), 0.0);
    }

    #[test]
    fn exp_log_pow_agree_with_known_constants() {
        assert_eq!(dexp(0.0), 1.0);
        assert!((dexp(1.0) - std::f64::consts::E).abs() < 1e-13);
        assert!((dexp(LN_2) - 2.0).abs() < 1e-13);
        assert!((dln(std::f64::consts::E) - 1.0).abs() < 1e-13);
        assert!((dln(2.0) - std::f64::consts::LN_2).abs() < 1e-13);
        assert!((dln(1.0 / 8.0) + 3.0 * std::f64::consts::LN_2).abs() < 1e-12);
        assert!((dpow(2.0, 10.0) - 1024.0).abs() < 1e-9);
        assert!((dpow(0.5, 1.6) - 0.329_876_977_693_223_5).abs() < 1e-12);
    }

    #[test]
    fn vec2_helpers() {
        let v = Vec2::new(3.0, 4.0);
        assert_eq!(v.len(), 5.0);
        assert_eq!(v.perp(), Vec2::new(-4.0, 3.0));
        assert_eq!(v.normalized().len(), 1.0);
        assert_eq!(clamp(2.0, 0.0, 1.0), 1.0);
        assert_eq!(lerp(1.0, 3.0, 0.25), 1.5);
    }
}
