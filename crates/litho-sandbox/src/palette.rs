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

/// Гипсометрическая палитра: абиссаль → шельф → суша → горы → снег.
pub fn hypsometric(e_km: f64) -> [u8; 3] {
    const STOPS: [(f64, [u8; 3]); 10] = [
        (-8.0, [6, 14, 48]),
        (-4.5, [16, 48, 112]),
        (-2.0, [32, 92, 168]),
        (-0.2, [86, 166, 202]),
        (0.0, [214, 205, 145]),
        (0.2, [96, 152, 82]),
        (1.0, [112, 132, 70]),
        (2.5, [146, 116, 84]),
        (4.5, [126, 112, 104]),
        (7.0, [242, 246, 250]),
    ];
    if e_km <= STOPS[0].0 {
        return STOPS[0].1;
    }
    for pair in STOPS.windows(2) {
        if e_km <= pair[1].0 {
            let t = (e_km - pair[0].0) / (pair[1].0 - pair[0].0);
            return mix(pair[0].1, pair[1].1, t);
        }
    }
    STOPS[STOPS.len() - 1].1
}

/// Поле материка κ: синий океан, зелёная суша, светлая береговая линия.
pub fn kappa(value_l: f64) -> [u8; 3] {
    if value_l < 0.0 {
        mix([18, 52, 112], [96, 176, 206], clamp01(1.0 + value_l / 6.0))
    } else {
        mix([196, 206, 150], [54, 128, 62], clamp01(value_l / 8.0))
    }
}

/// Горность M (красный), плато Pl (зелёный), удалённость ι (синий).
pub fn mpi(mountain: f64, plateau: f64, iota_l: f64) -> [u8; 3] {
    [
        (30.0 + 220.0 * clamp01(mountain)) as u8,
        (30.0 + 200.0 * clamp01(plateau)) as u8,
        (40.0 + 210.0 * clamp01(iota_l / 5.0)) as u8,
    ]
}

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
