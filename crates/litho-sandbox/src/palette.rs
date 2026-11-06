//! Свои палитры (план 12.3: не копируем чужие программы-карты).

use litho_core::hash::{splitmix64, unit};
use litho_core::math::clamp01;

pub fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let t = clamp01(t);
    let mut out = [0_u8; 3];
    for (i, channel) in out.iter_mut().enumerate() {
        let av = f64::from(a[i]);
        let bv = f64::from(b[i]);
        *channel = (av + (bv - av) * t) as u8;
    }
    out
}

/// Шум: холодные отрицательные значения, тёплые положительные.
pub fn noise(value: f64) -> [u8; 3] {
    mix([40, 70, 130], [220, 190, 120], (value + 1.0) * 0.5)
}

/// Диагностическая шкала: красный = плохо, зелёный = хорошо.
pub fn heat(value: f64) -> [u8; 3] {
    let v = clamp01(value);
    if v < 0.5 {
        mix([190, 30, 30], [235, 205, 80], v * 2.0)
    } else {
        mix([235, 205, 80], [50, 170, 90], (v - 0.5) * 2.0)
    }
}

/// Якобиан искажения: порог 0,2 из 7.3.2 показан красным.
pub fn jacobian(det: f64) -> [u8; 3] {
    if det <= 0.0 {
        [255, 0, 0]
    } else if det < 0.2 {
        mix([255, 0, 0], [240, 210, 90], det / 0.2)
    } else {
        mix([240, 210, 90], [40, 120, 200], clamp01((det - 0.2) / 0.8))
    }
}

pub fn from_hash(h: u64) -> [u8; 3] {
    let h1 = splitmix64(h);
    let h2 = splitmix64(h1);
    [
        50 + (unit(h) * 200.0) as u8,
        50 + (unit(h1) * 200.0) as u8,
        50 + (unit(h2) * 200.0) as u8,
    ]
}

pub const CONVERGENT: [u8; 3] = [220, 60, 60];
pub const DIVERGENT: [u8; 3] = [60, 110, 230];
pub const TRANSFORM: [u8; 3] = [70, 200, 110];

#[cfg(test)]
mod tests {
    use super::from_hash;

    #[test]
    fn hash_palette_uses_independent_channels() {
        let mut green_seen = false;
        let mut blue_seen = false;
        for h in 0..256_u64 {
            let c = from_hash(h ^ 0xDEAD_BEEF);
            if c[1] > 100 {
                green_seen = true;
            }
            if c[2] > 100 {
                blue_seen = true;
            }
            assert_ne!(c, [50, 50, 50]);
        }
        assert!(
            green_seen && blue_seen,
            "каналы не должны быть константными"
        );
    }
}
