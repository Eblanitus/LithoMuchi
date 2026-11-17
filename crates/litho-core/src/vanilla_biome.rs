//! Копия ванильной таблицы биомов поверхности (`OverworldBiomeBuilder`, 26.3).
//!
//! Игра выбирает биом не по нашему классификатору, а по пяти числам C/E/W/T/H
//! через эту таблицу. Здесь она переписана, чтобы ядро и песочница видели тот
//! же биом, что игра (Д-118). Таблица написана нами по устройству ванильного
//! построителя; файл Mojang не копируется. Сверка с отчётом datagen
//! `reports/biome_parameters/minecraft/overworld.json` — `litho-sandbox
//! vanilla-biomes --report`, в CI на настоящем сервере 26.3.
//!
//! Учитывается только поверхность (depth 0): подземные биомы (пещеры,
//! deep dark) пещер не касаются, их мы не трогаем.

use std::sync::OnceLock;

type Range = (f64, f64);

const FULL: Range = (-1.0, 1.0);
const T: [Range; 5] = [
    (-1.0, -0.45),
    (-0.45, -0.15),
    (-0.15, 0.2),
    (0.2, 0.55),
    (0.55, 1.0),
];
const H: [Range; 5] = [
    (-1.0, -0.35),
    (-0.35, -0.1),
    (-0.1, 0.1),
    (0.1, 0.3),
    (0.3, 1.0),
];
const E: [Range; 7] = [
    (-1.0, -0.78),
    (-0.78, -0.375),
    (-0.375, -0.2225),
    (-0.2225, 0.05),
    (0.05, 0.45),
    (0.45, 0.55),
    (0.55, 1.0),
];
const MUSHROOM: Range = (-1.2, -1.05);
const DEEP_OCEAN: Range = (-1.05, -0.455);
const OCEAN: Range = (-0.455, -0.19);
const COAST: Range = (-0.19, -0.11);
const NEAR: Range = (-0.11, 0.03);
const MID: Range = (0.03, 0.3);
const FAR: Range = (0.3, 1.0);

const fn span(a: Range, b: Range) -> Range {
    (a.0, b.1)
}

const OCEANS: [[&str; 5]; 2] = [
    [
        "deep_frozen_ocean",
        "deep_cold_ocean",
        "deep_ocean",
        "deep_lukewarm_ocean",
        "warm_ocean",
    ],
    [
        "frozen_ocean",
        "cold_ocean",
        "ocean",
        "lukewarm_ocean",
        "warm_ocean",
    ],
];
const MIDDLE: [[&str; 5]; 5] = [
    [
        "snowy_plains",
        "snowy_plains",
        "snowy_plains",
        "snowy_taiga",
        "taiga",
    ],
    [
        "plains",
        "plains",
        "forest",
        "taiga",
        "old_growth_spruce_taiga",
    ],
    [
        "flower_forest",
        "plains",
        "forest",
        "birch_forest",
        "dark_forest",
    ],
    ["savanna", "savanna", "forest", "jungle", "jungle"],
    ["desert", "desert", "desert", "desert", "desert"],
];
const MIDDLE_VARIANT: [[Option<&str>; 5]; 5] = [
    [Some("ice_spikes"), None, Some("snowy_taiga"), None, None],
    [
        Some("dappled_forest"),
        None,
        None,
        None,
        Some("old_growth_pine_taiga"),
    ],
    [
        Some("sunflower_plains"),
        None,
        None,
        Some("old_growth_birch_forest"),
        None,
    ],
    [
        None,
        None,
        Some("plains"),
        Some("sparse_jungle"),
        Some("bamboo_jungle"),
    ],
    [None, None, None, None, None],
];
const PLATEAU: [[&str; 5]; 5] = [
    [
        "snowy_plains",
        "snowy_plains",
        "snowy_plains",
        "snowy_taiga",
        "snowy_taiga",
    ],
    [
        "meadow",
        "meadow",
        "forest",
        "taiga",
        "old_growth_spruce_taiga",
    ],
    ["meadow", "meadow", "meadow", "meadow", "pale_garden"],
    [
        "savanna_plateau",
        "savanna_plateau",
        "forest",
        "forest",
        "jungle",
    ],
    [
        "badlands",
        "badlands",
        "badlands",
        "wooded_badlands",
        "wooded_badlands",
    ],
];
const PLATEAU_VARIANT: [[Option<&str>; 5]; 5] = [
    [Some("ice_spikes"), None, None, None, None],
    [
        Some("cherry_grove"),
        None,
        Some("meadow"),
        Some("meadow"),
        Some("old_growth_pine_taiga"),
    ],
    [
        Some("cherry_grove"),
        Some("cherry_grove"),
        Some("forest"),
        Some("birch_forest"),
        None,
    ],
    [None, None, None, None, None],
    [
        Some("eroded_badlands"),
        Some("eroded_badlands"),
        None,
        None,
        None,
    ],
];
const SHATTERED: [[Option<&str>; 5]; 5] = [
    [
        Some("windswept_gravelly_hills"),
        Some("windswept_gravelly_hills"),
        Some("windswept_hills"),
        Some("windswept_forest"),
        Some("windswept_forest"),
    ],
    [
        Some("windswept_gravelly_hills"),
        Some("windswept_gravelly_hills"),
        Some("windswept_hills"),
        Some("windswept_forest"),
        Some("windswept_forest"),
    ],
    [
        Some("windswept_hills"),
        Some("windswept_hills"),
        Some("windswept_hills"),
        Some("windswept_forest"),
        Some("windswept_forest"),
    ],
    [None, None, None, None, None],
    [None, None, None, None, None],
];

/// Срез W в ванили: проверка `weirdness.max() < 0` — по диапазону среза,
/// не по самой точке.
fn neg(w: Range) -> bool {
    w.1 < 0.0
}

fn middle(t: usize, h: usize, w: Range) -> &'static str {
    if neg(w) {
        MIDDLE[t][h]
    } else {
        MIDDLE_VARIANT[t][h].unwrap_or(MIDDLE[t][h])
    }
}

fn badlands(h: usize, w: Range) -> &'static str {
    if h < 2 {
        if neg(w) {
            "badlands"
        } else {
            "eroded_badlands"
        }
    } else if h < 3 {
        "badlands"
    } else {
        "wooded_badlands"
    }
}

fn middle_or_badlands(t: usize, h: usize, w: Range) -> &'static str {
    if t == 4 {
        badlands(h, w)
    } else {
        middle(t, h, w)
    }
}

fn middle_or_badlands_or_slope(t: usize, h: usize, w: Range) -> &'static str {
    if t == 0 {
        slope(t, h, w)
    } else {
        middle_or_badlands(t, h, w)
    }
}

fn windswept_savanna(t: usize, h: usize, w: Range, under: &'static str) -> &'static str {
    if t > 1 && h < 4 && w.1 >= 0.0 {
        "windswept_savanna"
    } else {
        under
    }
}

fn beach(t: usize) -> &'static str {
    match t {
        0 => "snowy_beach",
        4 => "desert",
        _ => "beach",
    }
}

fn shattered_coast(t: usize, h: usize, w: Range) -> &'static str {
    let b = if w.1 >= 0.0 {
        middle(t, h, w)
    } else {
        beach(t)
    };
    windswept_savanna(t, h, w, b)
}

fn plateau(t: usize, h: usize, w: Range) -> &'static str {
    if !neg(w) {
        if let Some(v) = PLATEAU_VARIANT[t][h] {
            return v;
        }
    }
    PLATEAU[t][h]
}

fn peak(t: usize, h: usize, w: Range) -> &'static str {
    if t <= 2 {
        if neg(w) {
            "jagged_peaks"
        } else {
            "frozen_peaks"
        }
    } else if t == 3 {
        "stony_peaks"
    } else {
        badlands(h, w)
    }
}

fn slope(t: usize, h: usize, w: Range) -> &'static str {
    if t >= 3 {
        plateau(t, h, w)
    } else if h <= 1 {
        "snowy_slopes"
    } else {
        "grove"
    }
}

fn shattered(t: usize, h: usize, w: Range) -> &'static str {
    SHATTERED[t][h].unwrap_or_else(|| middle(t, h, w))
}

/// Ячейка таблицы: диапазоны T, H, C, E, W и биом.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub t: Range,
    pub h: Range,
    pub c: Range,
    pub e: Range,
    pub w: Range,
    pub biome: &'static str,
}

struct Builder(Vec<Entry>);

impl Builder {
    fn add(&mut self, t: Range, h: Range, c: Range, e: Range, w: Range, biome: &'static str) {
        self.0.push(Entry {
            t,
            h,
            c,
            e,
            w,
            biome,
        });
    }

    fn each(&mut self, f: impl Fn(usize, usize) -> Vec<(Range, Range, &'static str)>) {
        for (ti, &tr) in T.iter().enumerate() {
            for (hi, &hr) in H.iter().enumerate() {
                for (c, e, b) in f(ti, hi) {
                    self.0.push(Entry {
                        t: tr,
                        h: hr,
                        c,
                        e,
                        w: (0.0, 0.0),
                        biome: b,
                    });
                }
            }
        }
    }

    /// `each` с диапазоном W среза.
    fn slice(
        &mut self,
        w: Range,
        f: impl Fn(usize, usize, Range) -> Vec<(Range, Range, &'static str)>,
    ) {
        let start = self.0.len();
        self.each(|t, h| f(t, h, w));
        for entry in &mut self.0[start..] {
            entry.w = w;
        }
    }

    fn off_coast(&mut self) {
        self.add(FULL, FULL, MUSHROOM, FULL, FULL, "mushroom_fields");
        for (t, &tr) in T.iter().enumerate() {
            self.add(tr, FULL, DEEP_OCEAN, FULL, FULL, OCEANS[0][t]);
            self.add(tr, FULL, OCEAN, FULL, FULL, OCEANS[1][t]);
        }
    }

    fn peaks(&mut self, w: Range) {
        self.slice(w, |t, h, w| {
            let sh = shattered(t, h, w);
            vec![
                (span(COAST, FAR), E[0], peak(t, h, w)),
                (
                    span(COAST, NEAR),
                    E[1],
                    middle_or_badlands_or_slope(t, h, w),
                ),
                (span(MID, FAR), E[1], peak(t, h, w)),
                (span(COAST, NEAR), span(E[2], E[3]), middle(t, h, w)),
                (span(MID, FAR), E[2], plateau(t, h, w)),
                (MID, E[3], middle_or_badlands(t, h, w)),
                (FAR, E[3], plateau(t, h, w)),
                (span(COAST, FAR), E[4], middle(t, h, w)),
                (span(COAST, NEAR), E[5], windswept_savanna(t, h, w, sh)),
                (span(MID, FAR), E[5], sh),
                (span(COAST, FAR), E[6], middle(t, h, w)),
            ]
        });
    }

    fn high(&mut self, w: Range) {
        self.slice(w, |t, h, w| {
            let sh = shattered(t, h, w);
            vec![
                (COAST, span(E[0], E[1]), middle(t, h, w)),
                (NEAR, E[0], slope(t, h, w)),
                (span(MID, FAR), E[0], peak(t, h, w)),
                (NEAR, E[1], middle_or_badlands_or_slope(t, h, w)),
                (span(MID, FAR), E[1], slope(t, h, w)),
                (span(COAST, NEAR), span(E[2], E[3]), middle(t, h, w)),
                (span(MID, FAR), E[2], plateau(t, h, w)),
                (MID, E[3], middle_or_badlands(t, h, w)),
                (FAR, E[3], plateau(t, h, w)),
                (span(COAST, FAR), E[4], middle(t, h, w)),
                (
                    span(COAST, NEAR),
                    E[5],
                    windswept_savanna(t, h, w, middle(t, h, w)),
                ),
                (span(MID, FAR), E[5], sh),
                (span(COAST, FAR), E[6], middle(t, h, w)),
            ]
        });
    }

    fn swamps_and_stony_shore(&mut self, w: Range) {
        self.add(FULL, FULL, COAST, span(E[0], E[2]), w, "stony_shore");
        self.add(span(T[1], T[2]), FULL, span(NEAR, FAR), E[6], w, "swamp");
        self.add(
            span(T[3], T[4]),
            FULL,
            span(NEAR, FAR),
            E[6],
            w,
            "mangrove_swamp",
        );
    }

    fn mid(&mut self, w: Range) {
        self.swamps_and_stony_shore(w);
        self.slice(w, |t, h, w| {
            let sh = shattered(t, h, w);
            let mut v = vec![
                (span(NEAR, FAR), E[0], slope(t, h, w)),
                (span(NEAR, MID), E[1], middle_or_badlands_or_slope(t, h, w)),
                (
                    FAR,
                    E[1],
                    if t == 0 {
                        slope(t, h, w)
                    } else {
                        plateau(t, h, w)
                    },
                ),
                (NEAR, E[2], middle(t, h, w)),
                (MID, E[2], middle_or_badlands(t, h, w)),
                (FAR, E[2], plateau(t, h, w)),
                (span(COAST, NEAR), E[3], middle(t, h, w)),
                (span(MID, FAR), E[3], middle_or_badlands(t, h, w)),
            ];
            if neg(w) {
                v.push((COAST, E[4], beach(t)));
                v.push((span(NEAR, FAR), E[4], middle(t, h, w)));
            } else {
                v.push((span(COAST, FAR), E[4], middle(t, h, w)));
            }
            v.push((COAST, E[5], shattered_coast(t, h, w)));
            v.push((NEAR, E[5], windswept_savanna(t, h, w, middle(t, h, w))));
            v.push((span(MID, FAR), E[5], sh));
            if neg(w) {
                v.push((COAST, E[6], beach(t)));
            } else {
                v.push((COAST, E[6], middle(t, h, w)));
            }
            if t == 0 {
                v.push((span(NEAR, FAR), E[6], middle(t, h, w)));
            }
            v
        });
    }

    fn low(&mut self, w: Range) {
        self.swamps_and_stony_shore(w);
        self.slice(w, |t, h, w| {
            let mut v = vec![
                (NEAR, span(E[0], E[1]), middle_or_badlands(t, h, w)),
                (
                    span(MID, FAR),
                    span(E[0], E[1]),
                    middle_or_badlands_or_slope(t, h, w),
                ),
                (NEAR, span(E[2], E[3]), middle(t, h, w)),
                (
                    span(MID, FAR),
                    span(E[2], E[3]),
                    middle_or_badlands(t, h, w),
                ),
                (COAST, span(E[3], E[4]), beach(t)),
                (span(NEAR, FAR), E[4], middle(t, h, w)),
                (COAST, E[5], shattered_coast(t, h, w)),
                (NEAR, E[5], windswept_savanna(t, h, w, middle(t, h, w))),
                (span(MID, FAR), E[5], middle(t, h, w)),
                (COAST, E[6], beach(t)),
            ];
            if t == 0 {
                v.push((span(NEAR, FAR), E[6], middle(t, h, w)));
            }
            v
        });
    }

    fn valleys(&mut self, w: Range) {
        let frozen = T[0];
        let unfrozen = span(T[1], T[4]);
        let shore_or = |r: &'static str| if neg(w) { "stony_shore" } else { r };
        self.add(
            frozen,
            FULL,
            COAST,
            span(E[0], E[1]),
            w,
            shore_or("frozen_river"),
        );
        self.add(
            unfrozen,
            FULL,
            COAST,
            span(E[0], E[1]),
            w,
            shore_or("river"),
        );
        self.add(frozen, FULL, NEAR, span(E[0], E[1]), w, "frozen_river");
        self.add(unfrozen, FULL, NEAR, span(E[0], E[1]), w, "river");
        self.add(
            frozen,
            FULL,
            span(COAST, FAR),
            span(E[2], E[5]),
            w,
            "frozen_river",
        );
        self.add(
            unfrozen,
            FULL,
            span(COAST, FAR),
            span(E[2], E[5]),
            w,
            "river",
        );
        self.add(frozen, FULL, COAST, E[6], w, "frozen_river");
        self.add(unfrozen, FULL, COAST, E[6], w, "river");
        self.add(span(T[1], T[2]), FULL, span(NEAR, FAR), E[6], w, "swamp");
        self.add(
            span(T[3], T[4]),
            FULL,
            span(NEAR, FAR),
            E[6],
            w,
            "mangrove_swamp",
        );
        self.add(frozen, FULL, span(NEAR, FAR), E[6], w, "frozen_river");
        self.slice(w, |t, h, w| {
            vec![(
                span(MID, FAR),
                span(E[0], E[1]),
                middle_or_badlands(t, h, w),
            )]
        });
    }

    fn inland(&mut self) {
        self.mid((-1.0, -0.933_333_3));
        self.high((-0.933_333_3, -0.766_666_7));
        self.peaks((-0.766_666_7, -0.566_666_66));
        self.high((-0.566_666_66, -0.4));
        self.mid((-0.4, -0.266_666_68));
        self.low((-0.266_666_68, -0.05));
        self.valleys((-0.05, 0.05));
        self.low((0.05, 0.266_666_68));
        self.mid((0.266_666_68, 0.4));
        self.high((0.4, 0.566_666_66));
        self.peaks((0.566_666_66, 0.766_666_7));
        self.high((0.766_666_7, 0.933_333_3));
        self.mid((0.933_333_3, 1.0));
    }
}

/// Таблица поверхности в порядке ванильного построителя.
pub fn entries() -> &'static [Entry] {
    static TABLE: OnceLock<Vec<Entry>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut b = Builder(Vec::new());
        b.off_coast();
        b.inland();
        b.0
    })
}

fn dist(r: Range, v: f64) -> f64 {
    if v < r.0 {
        r.0 - v
    } else if v > r.1 {
        v - r.1
    } else {
        0.0
    }
}

/// Биом, который выберет игра на поверхности (depth 0): ближайшая ячейка
/// по сумме квадратов выхода за диапазоны, как в ванили; при равенстве —
/// первая в порядке построителя.
pub fn surface_biome(c: f64, e: f64, w: f64, t: f64, h: f64) -> &'static str {
    let mut best = f64::INFINITY;
    let mut biome = "plains";
    for entry in entries() {
        let d = dist(entry.t, t).powi(2)
            + dist(entry.h, h).powi(2)
            + dist(entry.c, c).powi(2)
            + dist(entry.e, e).powi(2)
            + dist(entry.w, w).powi(2);
        if d < best {
            best = d;
            biome = entry.biome;
            if d == 0.0 {
                break;
            }
        }
    }
    biome
}

/// Холодные горные биомы ванили: снежные склоны, роща, ледяные пики.
pub fn is_cold_mountain(biome: &str) -> bool {
    matches!(
        biome,
        "snowy_slopes" | "grove" | "jagged_peaks" | "frozen_peaks"
    )
}

/// Горные биомы, которым место только на высокой горе (холодные + каменные пики).
pub fn is_mountain(biome: &str) -> bool {
    is_cold_mountain(biome) || biome == "stony_peaks"
}

/// Верх среза low: при |W| не выше этого ваниль на суше не ставит горных
/// биомов, кроме снежных склонов в снежном поясе (T ступени 0).
pub const LOW_SLICE_W: f64 = 0.25;

/// Д-118: горные биомы ванили (снежные склоны, роща, пики) только на высокой
/// горе. W в `v1-dev` решает только выбор биома (PV и высота свои), поэтому
/// его модуль ограничивается сверху: ниже `mountain_biome_y` — срезом low,
/// где ваниль ставит обычный биом (а снежному поясу — снежные склоны при
/// низкой E), выше `mountain_biome_full_y` — без ограничения. Между ними
/// предел растёт плавно, чтобы W оставался непрерывным (гейт seams 0,02).
pub fn guard_mountain_biomes(
    v: crate::vanilla::VanillaSample,
    shape_y: f64,
    params: &crate::params::Params,
) -> crate::vanilla::VanillaSample {
    let c = &params.climate;
    let s = crate::math::smoothstep(c.mountain_biome_y, c.mountain_biome_full_y, shape_y);
    let cap = LOW_SLICE_W + (1.0 - LOW_SLICE_W) * s;
    if v.w.abs() <= cap {
        return v;
    }
    crate::vanilla::VanillaSample {
        w: v.w.signum() * cap,
        ..v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_points() {
        // Лес умеренного пояса на равнине.
        assert_eq!(surface_biome(0.5, 0.1, -0.2, 0.0, 0.0), "forest");
        // Тот же климат на гребне: срез high, E1, far inland — роща (находка
        // игрового теста 2026-11-17, сид 42, X 750, Z −220).
        assert_eq!(surface_biome(0.64, -0.47, -0.42, 0.0, 0.0), "grove");
        assert_eq!(surface_biome(0.64, -0.47, -0.42, 0.0, -0.5), "snowy_slopes");
        // Пик: срез peaks, E0.
        assert_eq!(surface_biome(0.5, -0.9, -0.65, -0.7, 0.0), "jagged_peaks");
        assert_eq!(surface_biome(0.5, -0.9, 0.65, 0.3, 0.0), "stony_peaks");
        // Океан и река.
        assert_eq!(surface_biome(-0.6, 0.0, 0.0, 0.0, 0.0), "deep_ocean");
        assert_eq!(surface_biome(0.2, 0.1, 0.0, 0.0, 0.0), "river");
        assert_eq!(surface_biome(0.2, 0.1, 0.0, -0.7, 0.0), "frozen_river");
    }

    #[test]
    fn guard_keeps_forest_on_hill_and_snow_on_mountain() {
        use crate::mode::Mode;
        use crate::params::Params;
        use crate::vanilla::VanillaSample;
        use crate::version::GenVersion;
        let p = Params::defaults(GenVersion::V1Dev, Mode::Gameplay);
        let ridge = VanillaSample {
            c: 0.64,
            e: -0.47,
            w: -0.42,
            t: 0.0,
            h: 0.0,
        };
        let hill = guard_mountain_biomes(ridge, 140.0, &p);
        let b = surface_biome(hill.c, hill.e, hill.w, hill.t, hill.h);
        assert!(!is_mountain(b), "холм стал {b}");
        assert_eq!(
            (hill.c, hill.e, hill.t, hill.h),
            (ridge.c, ridge.e, ridge.t, ridge.h)
        );
        // Снежный пояс на холме при низкой E — снежные склоны остаются.
        let snowy = VanillaSample { t: -0.7, ..hill };
        let b = surface_biome(snowy.c, snowy.e, snowy.w, snowy.t, snowy.h);
        assert!(is_cold_mountain(b), "снежный пояс стал {b}");
        // На высокой горе ваниль решает сама.
        let top = p.climate.mountain_biome_full_y;
        assert_eq!(guard_mountain_biomes(ridge, top, &p), ridge);
        // Непрерывность: предел меняется плавно с высотой.
        let mut prev = guard_mountain_biomes(VanillaSample { w: 0.9, ..ridge }, 0.0, &p).w;
        for y in 1..400 {
            let w = guard_mountain_biomes(VanillaSample { w: 0.9, ..ridge }, y as f64, &p).w;
            assert!((w - prev).abs() < 0.012, "скачок W на y={y}");
            prev = w;
        }
    }

    #[test]
    fn table_covers_every_inland_cell() {
        // Каждая точка суши попадает ровно в ячейку (расстояние 0).
        let mut n = 0;
        for ci in 0..20 {
            for ei in 0..20 {
                for wi in 0..40 {
                    for tr in &T {
                        for hr in &H {
                            let c = -0.105 + 1.1 * ci as f64 / 20.0;
                            let e = -0.995 + 1.99 * ei as f64 / 20.0;
                            let w = -0.995 + 1.99 * wi as f64 / 40.0;
                            let t = tr.0 + 0.01;
                            let h = hr.0 + 0.01;
                            let hit = entries().iter().any(|en| {
                                dist(en.t, t) == 0.0
                                    && dist(en.h, h) == 0.0
                                    && dist(en.c, c) == 0.0
                                    && dist(en.e, e) == 0.0
                                    && dist(en.w, w) == 0.0
                            });
                            assert!(hit, "дыра в таблице: c={c} e={e} w={w} t={t} h={h}");
                            n += 1;
                        }
                    }
                }
            }
        }
        assert!(n > 0);
    }
}
