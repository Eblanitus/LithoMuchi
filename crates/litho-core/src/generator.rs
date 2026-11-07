//! Точка входа в ядро (эскиз публичного API 9.4).
//!
//! Здесь собраны примитивы, плиты, кора, климат, классификация и пять
//! ванильных параметров. Есть эталонный путь (`sample_reference`) и быстрый
//! путь через грубую решётку с кэшем тайлов (`sample`), плюс пять полей для
//! игры (`field`). Песочница и будущий мод вызывают один и тот же `Generator`.

use std::cell::RefCell;
use std::sync::Arc;

use crate::cache::{CACHE_CAPACITY, ShardedCache};
use crate::classify::{ClassSample, classify};
use crate::climate::{ClimateSample, PrecipSample, SurfaceSample, TemperatureSample, dryness};
use crate::crust::{CrustSample, crust_from_sample};
use crate::field::{Field, FullSample, to_f32};
use crate::lattice::{self, Nodes};
use crate::math::Vec2;
use crate::mode::Mode;
use crate::params::Params;
use crate::plates::{PlateSample, plate_query, warp_jacobian};
use crate::rivers::{RiverCache, local_r, traced_r};
use crate::seed::from_world_seed;
use crate::spawn::{self, SpawnQuality};
use crate::vanilla::{VanillaSample, continentalness};
use crate::version::GenVersion;

/// Полей в узле грубой решётки: φ, SST и две компоненты ветра (Д-021).
/// Осадки и влагосодержание больше НЕ кэшируются: это результат луча влаги
/// длиной 8 L, который выходит за пределы coarse-тайла в 5 L и при рябистом
/// береге не интерполируется. Кэш хранит только плавные поля.
const COARSE_N: usize = 4;

type CoarseTiles = ShardedCache<(i64, i64), Arc<Nodes<COARSE_N>>>;
type LastCoarse = Option<(u64, u64, i64, i64, Arc<Nodes<COARSE_N>>)>;

thread_local! {
    /// «Последний тайл» на поток: соседние запросы не трогают замки (6.6).
    /// Ключ включает сид и масштаб, чтобы миры в одном процессе не путались (Р14).
    static LAST_COARSE: RefCell<LastCoarse> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenError {
    /// Версия ещё не реализована (v1 появится при заморозке T90).
    UnsupportedVersion(GenVersion),
}

impl std::fmt::Display for GenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(v) => write!(f, "версия генератора {v:?} не реализована"),
        }
    }
}

impl std::error::Error for GenError {}

/// Всё, что известно о точке мира после слоёв L2, L3, L5 и L6 (T30–T43).
#[derive(Clone, Debug)]
pub struct WorldSample {
    pub climate: ClimateSample,
    pub class: ClassSample,
    pub vanilla: VanillaSample,
    /// Сила рифтовой/предгорной реки, вошедшая в W (7.9.3).
    pub river: f64,
    pub is_ocean: bool,
}

#[derive(Clone, Debug)]
pub struct Generator {
    seed: u64,
    params: Params,
    rivers: Arc<RiverCache>,
    coarse: Arc<CoarseTiles>,
    spawn_shift: Vec2,
}

impl Generator {
    /// Создаётся один раз на мир. Пока дёшево; после T54 здесь появится поиск
    /// сдвига точки появления `p*` (около 256 эталонных расчётов).
    pub fn new(seed: i64, mode: Mode, version: GenVersion) -> Result<Self, GenError> {
        if version != GenVersion::V0 {
            return Err(GenError::UnsupportedVersion(version));
        }
        let world_seed = from_world_seed(seed);
        let params = Params::defaults(version, mode);
        // T54: точка появления ищется один раз на мир, только по сиду и версии.
        let shift = spawn::find_shift(world_seed, &params);
        let shards = params.lattice.cache_shards.max(1);
        Ok(Self {
            seed: world_seed,
            params,
            rivers: Arc::new(RiverCache::new()),
            coarse: Arc::new(ShardedCache::new(shards, CACHE_CAPACITY)),
            spawn_shift: shift,
        })
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub const fn params(&self) -> &Params {
        &self.params
    }

    /// Сдвиг макро-координат `p*` (7.12). В v0 найден и доступен слою `spawn`,
    /// но ещё не подставлен в `macro_point`: карты песочницы остаются на месте
    /// до T55, чтобы автор сравнивал их с прежними (Д-015).
    pub fn spawn_shift(&self) -> Vec2 {
        self.spawn_shift
    }

    /// Блочные координаты → макро-координата `p = x/L, z/L`.
    /// Сдвиг `p*` (7.12) в v0 найден, но не подставлен сюда (Д-015): карты
    /// песочницы остаются на прежнем месте планеты, слой `spawn` центрируется
    /// на `p*` сам.
    pub fn macro_point(&self, x: f64, z: f64) -> Vec2 {
        let l = self.params.block.l_blocks;
        Vec2::new(x / l, z / l)
    }

    /// Плита и границы в блочной точке.
    pub fn plate_sample(&self, x: f64, z: f64) -> Option<PlateSample> {
        plate_query(self.macro_point(x, z), self.seed, &self.params)
    }

    /// Кора и макрорельеф в блочной точке (T22–T24).
    pub fn crust_sample(&self, x: f64, z: f64) -> Option<CrustSample> {
        let point = self.macro_point(x, z);
        let sample = plate_query(point, self.seed, &self.params)?;
        Some(crust_from_sample(point, &sample, self.seed, &self.params))
    }

    /// Диагностика искажения домена (слой `jacobian`).
    pub fn jacobian(&self, x: f64, z: f64) -> f64 {
        warp_jacobian(self.macro_point(x, z), self.seed, &self.params)
    }

    /// Поверхность в произвольной macro-точке: нужно линии тока влаги (7.7).
    fn surface_at(&self, point: Vec2) -> SurfaceSample {
        match plate_query(point, self.seed, &self.params) {
            Some(sample) => {
                let crust = crust_from_sample(point, &sample, self.seed, &self.params);
                SurfaceSample {
                    e_km: crust.e_km,
                    iota_l: crust.iota_l,
                    mountain: crust.mountain,
                    plateau: crust.plateau,
                    boundary_weights: crust.boundary_weights,
                    sigma_l: sample.d_ab_l,
                }
            }
            None => SurfaceSample::default(),
        }
    }

    /// Климат в блочной точке (T30–T33).
    pub fn climate_sample(&self, x: f64, z: f64) -> Option<ClimateSample> {
        let point = self.macro_point(x, z);
        crate::climate::sample(point, self.seed, &self.params, &|q| self.surface_at(q))
    }

    /// Климат, жизненная зона и пять ванильных чисел (T40–T43).
    pub fn world_sample(&self, x: f64, z: f64) -> Option<WorldSample> {
        let climate = self.climate_sample(x, z)?;
        let c = continentalness(climate.surface.e_km, climate.surface.iota_l, &self.params);
        let is_ocean = c < self.params.climate.ocean_c_threshold;
        let class = classify(&climate, is_ocean, x, z, self.seed, &self.params);
        // Дешёвая часть рек входит в W везде; трассированные реки считаются
        // отдельно для слоя `rivers` (Д-015).
        let river = local_r(x, z, &climate.surface, self.seed, &self.params);
        let vanilla =
            crate::vanilla::sample(&climate, &class, x, z, self.seed, &self.params, river);
        Some(WorldSample {
            climate,
            class,
            vanilla,
            river,
            is_ocean,
        })
    }

    /// Полная сила реки вместе с трассированными руслами (слой `rivers`).
    pub fn river_sample(&self, x: f64, z: f64) -> f64 {
        let point = self.macro_point(x, z);
        let surface = self.surface_at(point);
        let local = local_r(x, z, &surface, self.seed, &self.params);
        let traced = traced_r(
            point,
            surface.e_km,
            self.seed,
            &self.params,
            &|q| self.surface_at(q),
            &self.rivers,
        );
        local.max(traced)
    }

    /// Поле глубинной лавы λ (слой `lava`, T53).
    pub fn lava_sample(&self, x: f64, z: f64) -> f64 {
        let point = self.macro_point(x, z);
        match plate_query(point, self.seed, &self.params) {
            Some(sample) => crate::lava::sample(point, &sample, self.seed, &self.params),
            None => self.params.lava.background,
        }
    }

    /// Насколько точка подходит для появления (слой `spawn`, T54).
    pub fn spawn_quality(&self, x: f64, z: f64) -> SpawnQuality {
        spawn::evaluate(self.macro_point(x, z), self.seed, &self.params)
    }
    /// Эталонный путь (14.2): чистая функция, без решётки и кэшей. Истина для
    /// всех проверок и для карт песочницы в режиме `reference`.
    pub fn sample_reference(&self, x: f64, z: f64) -> Option<FullSample> {
        let world = self.world_sample(x, z)?;
        Some(self.full_from_world(x, z, world))
    }

    fn full_from_world(&self, x: f64, z: f64, world: WorldSample) -> FullSample {
        FullSample {
            point: self.macro_point(x, z),
            plate: self.plate_sample(x, z),
            zone: world.class.zone,
            cell: world.class.cell,
            climate: world.climate,
            class: world.class,
            vanilla: world.vanilla,
            lava: self.lava_sample(x, z),
            world,
        }
    }

    /// Грубый тайл: климат в 20×20 узлах, сглаживание [1,4,6,4,1]/16.
    /// Высота в точках луча влаги берётся эталонной функцией рельефа (Д-017:
    /// на T79 её заменит быстрая решётка высот).
    fn compute_coarse_tile(&self, tx: i64, tz: i64) -> Arc<Nodes<COARSE_N>> {
        let step = self.params.block.delta_coarse_blocks;
        let l = self.params.block.l_blocks;
        let mut values: Nodes<COARSE_N> = vec![[0.0; COARSE_N]; lattice::CELLS];
        for i in 0..lattice::NODES {
            for j in 0..lattice::NODES {
                let (bx, bz) = lattice::node_coords(tx, tz, i, j, step);
                let point = Vec2::new(bx / l, bz / l);
                let cell = match crate::climate::pole_sample(point, self.seed, &self.params) {
                    // Только плавные поля: полюса, SST и ветер. Луч влаги
                    // трассируется позже, из конкретной точки (Д-021).
                    Some(pole) => {
                        let surface = self.surface_at(point);
                        let t = crate::climate::temperature(
                            pole.phi_deg,
                            surface.e_km,
                            0.5,
                            &self.params,
                        );
                        let w =
                            crate::climate::wind(pole.phi_deg, pole.north, pole.east, &self.params);
                        [pole.phi_deg, t.sst_c, w.x, w.z]
                    }
                    // Полюс не нашёлся: нейтральные значения, не NaN.
                    None => [45.0, 15.0, 0.0, 1.0],
                };
                values[i * lattice::NODES + j] = cell;
            }
        }
        lattice::smooth(&mut values);
        Arc::new(values)
    }

    fn coarse_tile_at(&self, tx: i64, tz: i64) -> Arc<Nodes<COARSE_N>> {
        let scale = self.params.block.l_blocks.to_bits();
        let cached = LAST_COARSE.with(|cell| {
            cell.borrow().as_ref().and_then(|(seed, s, x, z, tile)| {
                if *seed == self.seed && *s == scale && *x == tx && *z == tz {
                    Some(tile.clone())
                } else {
                    None
                }
            })
        });
        if let Some(tile) = cached {
            return tile;
        }
        let tile = self
            .coarse
            .get_or_insert((tx, tz), || self.compute_coarse_tile(tx, tz));
        LAST_COARSE.with(|cell| {
            *cell.borrow_mut() = Some((self.seed, scale, tx, tz, tile.clone()));
        });
        tile
    }

    fn coarse_values(&self, x: f64, z: f64) -> [f64; COARSE_N] {
        let step = self.params.block.delta_coarse_blocks;
        let (tx, tz) = lattice::tile_key(x, z, step);
        let tile = self.coarse_tile_at(tx, tz);
        lattice::catmull_rom(
            &tile,
            lattice::local(x, tx, step),
            lattice::local(z, tz, step),
        )
    }

    /// Плавные поля coarse-решётки для диагностики и этапа Э2 (Д-021).
    /// Луч влаги их больше не использует: точный ветер дешевле и совпадает
    /// с эталоном (Д-025).
    pub fn coarse_probe(&self, x: f64, z: f64) -> [f64; COARSE_N] {
        self.coarse_values(x, z)
    }

    /// Быстрый путь (T55): рельеф, реки, лава и пять ванильных чисел считаются
    /// точно в блоке, а дорогой климат берётся интерполяцией по грубой решётке.
    pub fn sample(&self, x: f64, z: f64) -> Option<FullSample> {
        let plates = self.plate_sample(x, z)?;
        let point = self.macro_point(x, z);
        let crust = crust_from_sample(point, &plates, self.seed, &self.params);
        let surface = SurfaceSample {
            e_km: crust.e_km,
            iota_l: crust.iota_l,
            mountain: crust.mountain,
            plateau: crust.plateau,
            boundary_weights: crust.boundary_weights,
            sigma_l: plates.d_ab_l,
        };
        // Д-021: φ и направление ветра берутся точными в самой точке, иначе
        // t_bio дрожит на ±0,4 °C и перескакивает через порог пояса 18 °C.
        // coarse-решётка экономит только на 32 шагах линии тока влаги.
        let pole = crate::climate::pole_sample(point, self.seed, &self.params)?;
        let wind = crate::climate::wind(pole.phi_deg, pole.north, pole.east, &self.params);
        let wind_len = wind.len();
        let dir = if wind_len > 1e-9 {
            wind.normalized()
        } else {
            crate::climate::fallback_dir(&self.params)
        };
        // Д-025: профиль показал, что 33 точных `pole_sample` стоят 23 мкс,
        // а 33 пересчёта рельефа в луче — около 850 мкс. Интерполяция ветра
        // по coarse-решётке экономила 2,5 % времени, но при влажном климате
        // роняла совпадение зон до 0,953. Берём ветер точно, как в эталоне:
        // это дешевле, чем ослаблять инвариант 14.2.
        let wind_fn = |q: Vec2| crate::climate::wind_at(q, self.seed, &self.params);
        let ray =
            crate::climate::trace_ray(point, &self.params, &|q| self.surface_at(q), dir, &wind_fn);
        let c = &self.params.climate;
        let c_iso = crate::math::smoothstep(0.0, c.c_iso_end_l, surface.iota_l);
        let cont = crate::math::lerp(
            c_iso,
            ray.cont_ray,
            crate::math::smoothstep(c.c_ray_wind_lo, c.c_ray_wind_hi, wind_len),
        );
        let temperature =
            crate::climate::temperature(pole.phi_deg, surface.e_km, cont, &self.params);
        let sst_c = temperature.sst_c;
        let p_mm =
            crate::climate::precipitation(pole.phi_deg, ray.mu, wind_len, ray.slope, &self.params);
        let (pet_mm, ratio) = dryness(temperature.t_bio, p_mm, &self.params);
        let climate = ClimateSample {
            point,
            surface,
            pole,
            wind,
            temperature: TemperatureSample {
                t_c: temperature.t_c,
                sst_c,
                amp_c: temperature.amp_c,
                t_bio: temperature.t_bio,
                t_min: temperature.t_min,
            },
            precip: PrecipSample {
                mu: ray.mu,
                cont,
                p_mm,
                pet_mm,
                ratio,
                slope: ray.slope,
            },
        };
        let c_value = continentalness(surface.e_km, surface.iota_l, &self.params);
        let is_ocean = c_value < self.params.climate.ocean_c_threshold;
        let class = classify(&climate, is_ocean, x, z, self.seed, &self.params);
        let river = local_r(x, z, &surface, self.seed, &self.params);
        let vanilla =
            crate::vanilla::sample(&climate, &class, x, z, self.seed, &self.params, river);
        let world = WorldSample {
            climate,
            class,
            vanilla,
            river,
            is_ocean,
        };
        let mut full = self.full_from_world(x, z, world);
        full.plate = Some(plates);
        Some(full)
    }

    /// `WorldSample` быстрого пути — для слоёв песочницы с `--path fast`.
    pub fn world_sample_fast(&self, x: f64, z: f64) -> Option<WorldSample> {
        self.sample(x, z).map(|full| full.world)
    }

    /// Одно ванильное поле в блочных координатах: то, что зовёт мод (9.4).
    pub fn field(&self, field: Field, x: f64, _y: f64, z: f64) -> f64 {
        let Some(full) = self.sample(x, z) else {
            return 0.0;
        };
        let value = match field {
            Field::Continents => full.vanilla.c,
            Field::Erosion => full.vanilla.e,
            Field::Ridges => full.vanilla.w,
            Field::Temperature => full.vanilla.t,
            Field::Vegetation => full.vanilla.h,
            Field::Lava => full.lava,
            Field::Offset => full.climate.surface.e_km,
        };
        if value.is_finite() { value } else { 0.0 }
    }

    /// Значение на колонку `(x, z)` для `add_to_volume` (10.3).
    pub fn field_column(&self, field: Field, x: f64, z: f64) -> f64 {
        self.field(field, x, 0.0, z)
    }

    /// Граница с Horizon: внутри ядра всё `f64`, наружу отдаётся `f32`.
    pub fn field_f32(&self, field: Field, x: f64, y: f64, z: f64) -> f32 {
        to_f32(self.field(field, x, y, z))
    }

    /// Наибольшие скачки C/E/W между соседними блоками (команда `seams`).
    pub fn seam_deltas(&self, field: Field, x: f64, z: f64, step: f64) -> (f64, f64, f64) {
        let at = |dx: f64, dz: f64| {
            let sample = self.sample(x + dx, z + dz);
            sample.map_or(0.0, |full| match field {
                Field::Continents => full.vanilla.c,
                Field::Erosion => full.vanilla.e,
                Field::Ridges => full.vanilla.w,
                _ => full.vanilla.c,
            })
        };
        let here = at(0.0, 0.0);
        (
            (at(step, 0.0) - here).abs(),
            (at(0.0, step) - here).abs(),
            (at(step, step) - here).abs(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{GenError, Generator};
    use crate::mode::Mode;
    use crate::version::GenVersion;

    #[test]
    fn v1_is_not_implemented_yet() {
        assert_eq!(
            Generator::new(1, Mode::Gameplay, GenVersion::V1).unwrap_err(),
            GenError::UnsupportedVersion(GenVersion::V1)
        );
    }

    #[test]
    fn world_sample_is_finite_and_bounded() {
        let generator = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        for k in 0..12 {
            let x = k as f64 * 137.0 - 800.0;
            let z = k as f64 * -83.0 + 400.0;
            let w = generator.world_sample(x, z).unwrap();
            assert!(w.climate.temperature.t_bio.is_finite());
            assert!(w.climate.precip.p_mm.is_finite() && w.climate.precip.p_mm >= 0.0);
            assert!(w.climate.precip.ratio.is_finite() && w.climate.precip.ratio > 0.0);
            for value in [
                w.vanilla.c,
                w.vanilla.e,
                w.vanilla.w,
                w.vanilla.t,
                w.vanilla.h,
            ] {
                assert!(
                    (-1.0..=1.0).contains(&value),
                    "значение {value} вне [-1, 1]"
                );
            }
        }
    }

    #[test]
    fn macro_coordinates_scale_with_mode() {
        let sci = Generator::new(42, Mode::Scientific, GenVersion::V0).unwrap();
        let game = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        let a = sci.macro_point(4000.0, -2000.0);
        let b = game.macro_point(1000.0, -500.0);
        assert_eq!(a, b);
    }
}
