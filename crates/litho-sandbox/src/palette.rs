//! Свои палитры (план 12.3: не копируем чужие программы-карты).

use litho_core::hash::{fnv1a64, splitmix64, unit};
use litho_core::math::{Vec2, clamp01, dln};

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

/// Псевдоширота: тёплый экватор → белые полярные шапки, изолинии через 10°.
pub fn latitude(phi_deg: f64) -> [u8; 3] {
    let base = mix([214, 126, 62], [232, 242, 252], clamp01(phi_deg / 90.0));
    let band = (phi_deg / 10.0).fract();
    if band < 0.035 || band > 0.965 {
        mix(base, [255, 255, 255], 0.45)
    } else {
        base
    }
}

/// Биотемпература: синий холод → красное тепло.
pub fn thermal(t_c: f64) -> [u8; 3] {
    mix([58, 92, 190], [222, 92, 44], clamp01((t_c + 25.0) / 55.0))
}

/// Ветер: направление двумя каналами, сила синим. Стрелки не рисуем — слой
/// показывает три пояса и штиль у экватора оттенком.
pub fn wind_field(v: Vec2, strength: f64) -> [u8; 3] {
    [
        (255.0 * clamp01((v.x + 1.0) * 0.5)) as u8,
        (255.0 * clamp01((v.z + 1.0) * 0.5)) as u8,
        (255.0 * clamp01(strength)) as u8,
    ]
}

/// Влага: синий — влажный воздух, рыжий — континентальность.
pub fn moisture_field(mu: f64, cont: f64) -> [u8; 3] {
    let wet = mix([186, 152, 92], [28, 138, 202], clamp01(mu / 1.25));
    mix(wet, [214, 132, 70], 0.5 * clamp01(cont))
}

/// Осадки: логарифмическая шкала, рыжий оттенок — сухие провинции.
pub fn precip_field(p_mm: f64, ratio: f64) -> [u8; 3] {
    let wet = clamp01(dln(p_mm.max(10.0)) / dln(3000.0));
    let base = mix([196, 164, 96], [32, 104, 196], wet);
    if ratio > 1.0 {
        mix(base, [226, 178, 86], clamp01((ratio - 1.0) / 4.0))
    } else {
        base
    }
}

/// Жизненные зоны: свой набор цветов поясов, сухость осветляет к песку.
pub fn zone_field(belt: u8, province: u8, is_ocean: bool) -> [u8; 3] {
    const BELTS: [[u8; 3]; 7] = [
        [232, 240, 252],
        [188, 206, 222],
        [112, 166, 152],
        [86, 158, 92],
        [148, 180, 82],
        [196, 164, 74],
        [58, 138, 84],
    ];
    if is_ocean {
        return [22, 62, 124];
    }
    let base = BELTS[(belt as usize).min(6)];
    let wetness = clamp01((f64::from(province) - 1.0) / 5.0);
    mix([216, 188, 128], base, wetness)
}

/// Целевой ванильный биом: устойчивый цвет от хэша названия.
pub fn biome_field(name: &str) -> [u8; 3] {
    from_hash(fnv1a64(name))
}

/// C, E, W тремя каналами: резких скачков быть не должно.
pub fn vanilla_field(c: f64, e: f64, w: f64) -> [u8; 3] {
    [
        (255.0 * clamp01((c + 1.0) * 0.5)) as u8,
        (255.0 * clamp01((e + 1.0) * 0.5)) as u8,
        (255.0 * clamp01((w + 1.0) * 0.5)) as u8,
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
