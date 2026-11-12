//! Точка входа в ядро (эскиз публичного API 9.4).
//!
//! Здесь собраны примитивы, плиты, кора, климат, классификация и пять
//! ванильных параметров. Есть эталонный путь (`sample_reference`) и быстрый
//! путь через грубую решётку с кэшем тайлов (`sample`), плюс пять полей для
//! игры (`field`). Песочница и будущий мод вызывают один и тот же `Generator`.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cache::{CACHE_CAPACITY, ShardedCache};
use crate::classify::{ClassSample, classify};
use crate::climate::{ClimateSample, PrecipSample, SurfaceSample, TemperatureSample, dryness};
use crate::crust::{CrustSample, crust_from_sample};
use crate::drainage::{DrainageInput, DrainageOutlet, RiverNet, RiverNetCache, RiverNetError};
use crate::field::{Field, FullSample, to_f32};
use crate::hydrology::{self, HydrologySample};
use crate::lattice::{self, Nodes};
use crate::math::Vec2;
use crate::mode::Mode;
use crate::morphoclimate::{self, MorphoWeights};
use crate::params::Params;
use crate::plates::{Plate, PlateSample, plate_query, plates_near, warp_jacobian};
use crate::relief::{self, ReliefSample};
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
/// T160: кэш колонок для `field()` — игра спрашивает несколько полей в одной
/// колонке. Ёмкость инфраструктурная, в хеш мира не входит; вытеснение меняет
/// только скорость, не ответ.
const COLUMN_CACHE_CAPACITY: usize = 4096;
/// Д-059: кэш лучей влаги в узлах решётки быстрого пути. Узел — два десятка
/// чисел; при шаге 32 блока ёмкости хватает на область 4096×4096 блоков.
/// В хеш мира не входит: вытеснение меняет только скорость, не ответ.
const RAY_CACHE_CAPACITY: usize = 16384;
/// К2/К7: `field()` раньше молча подменял ошибку и нечисловое значение нулём.
/// litho-core не имеет зависимостей (план 5.4), поэтому пишем в stderr без
/// внешнего `log`-крейта; первые ошибки видны в логе игры/CI, далее только
/// счётчик, чтобы ошибка в каждой колонке не заваливала лог.
const FIELD_LOG_LIMIT: u64 = 16;
static FIELD_ERRORS: AtomicU64 = AtomicU64::new(0);

/// Сколько раз `field()` подменил ошибку/нечисловое значение нулём.
/// Диагностика для игры и тестов; не входит в хеш параметров.
pub fn field_error_log_count() -> u64 {
    FIELD_ERRORS.load(Ordering::Relaxed)
}

fn log_field_error(message: impl FnOnce() -> String) {
    log_field_error_with(&FIELD_ERRORS, message);
}

fn log_field_error_with(counter: &AtomicU64, message: impl FnOnce() -> String) {
    let number = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if number <= FIELD_LOG_LIMIT {
        eprintln!(
            "litho-core: {}; field() возвращает 0.0 (ошибка {number})",
            message()
        );
    } else if number == FIELD_LOG_LIMIT + 1 {
        eprintln!(
            "litho-core: дальше сообщения field() подавляются; всего ошибок см. field_error_log_count()"
        );
    }
}

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
    /// Сила реки в W: v0 — локальная плюс трассы; v1-dev — сеть T115
    /// плюс рифт/трансформ T113. Внутреннее море идёт через C, болото через E.
    pub river: f64,
    pub hydrology: HydrologySample,
    pub is_ocean: bool,
    /// T141: собственный PV для I2-lite. В v0 это прежняя производная f(W).
    pub pv: f64,
    /// T144: собственная высота/factor/jaggedness (полная I2, только v1-dev).
    /// В v0 поле не используется паком и остаётся диагностическим значением
    /// по умолчанию, чтобы не менять замороженные пять полей.
    pub height: crate::height::HeightSample,
}

/// Результат перевода колонки в игровые поля (T115/T122/T141/T144).
struct SurfaceFields {
    class: crate::classify::ClassSample,
    vanilla: VanillaSample,
    river: f64,
    hydrology: HydrologySample,
    is_ocean: bool,
    pv: f64,
    height: crate::height::HeightSample,
}

#[derive(Clone, Debug)]
pub struct Generator {
    seed: u64,
    params: Params,
    rivers: Arc<RiverCache>,
    hydrology: Arc<crate::cache::ShardedCache<(i64, i64), Arc<HydrologySample>>>,
    /// T122: региональные веса морфозон для рельефа. Кэш не меняет ответ:
    /// узлы привязаны к мировым блокам, эталон считает ту же решётку.
    morpho: Arc<crate::cache::ShardedCache<(i64, i64), MorphoWeights>>,
    drainage: Arc<RiverNetCache>,
    coarse: Arc<CoarseTiles>,
    /// T160: точная колонка `(x,z)` для повторных запросов полей.
    column: Arc<crate::cache::ShardedCache<(i64, i64), Arc<FullSample>>>,
    /// Д-059: лучи влаги в узлах решётки быстрого пути (только v1-dev).
    rays: Arc<crate::cache::ShardedCache<(i64, i64), Option<crate::climate::RayNode>>>,
    /// Д-059: счётчик посчитанных узлов, только для диагностики.
    ray_traces: Arc<std::sync::atomic::AtomicU64>,
    spawn_shift: Vec2,
}

impl Generator {
    /// Создаётся один раз на мир. Пока дёшево; после T54 здесь появится поиск
    /// сдвига точки появления `p*` (около 256 эталонных расчётов).
    pub fn new(seed: i64, mode: Mode, version: GenVersion) -> Result<Self, GenError> {
        match version {
            GenVersion::V0 | GenVersion::V1Dev => {}
            GenVersion::V1 => return Err(GenError::UnsupportedVersion(version)),
        }
        let world_seed = from_world_seed(seed);
        let params = Params::defaults(version, mode);
        if version.is_water_relief_dev() {
            debug_assert!(
                params.boundaries.validate().is_ok(),
                "параметры границ v1-dev"
            );
            debug_assert!(
                crate::height::validate(&params).is_ok(),
                "параметры собственной высоты v1-dev"
            );
        }
        // T54: точка появления ищется один раз на мир, только по сиду и версии.
        let shift = spawn::find_shift(world_seed, &params);
        let shards = params.lattice.cache_shards.max(1);
        Ok(Self {
            seed: world_seed,
            params,
            rivers: Arc::new(RiverCache::new()),
            hydrology: Arc::new(crate::cache::ShardedCache::new(shards, 4096)),
            morpho: Arc::new(crate::cache::ShardedCache::new(shards, 4096)),
            drainage: Arc::new(RiverNetCache::new(
                shards,
                crate::drainage::NET_CACHE_CAPACITY,
            )),
            coarse: Arc::new(ShardedCache::new(shards, CACHE_CAPACITY)),
            column: Arc::new(crate::cache::ShardedCache::new(
                shards,
                COLUMN_CACHE_CAPACITY,
            )),
            rays: Arc::new(crate::cache::ShardedCache::new(shards, RAY_CACHE_CAPACITY)),
            ray_traces: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            spawn_shift: shift,
        })
    }

    pub const fn seed(&self) -> u64 {
        self.seed
    }

    pub const fn params(&self) -> &Params {
        &self.params
    }

    /// Сдвиг макро-координат `p*` (7.12). Подставляется в `macro_point`,
    /// поэтому точка появления попадает в заданную клетку (Д-015 закрыт).
    pub fn spawn_shift(&self) -> Vec2 {
        self.spawn_shift
    }

    /// Блочные координаты → макро-координата `p = x/L, z/L`.
    /// Сдвиг `p*` (7.12) подставлен: весь мир сдвигается так, чтобы точка
    /// появления попала в заданную клетку (Д-015 закрыт).
    pub fn macro_point(&self, x: f64, z: f64) -> Vec2 {
        let l = self.params.block.l_blocks;
        Vec2::new(x / l, z / l).add(self.spawn_shift)
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

    /// Потенциал рельефа T120 в блочной точке. Ошибки сети стока не
    /// маскируются под нулевую котловину; игровой `field` пока не меняется.
    pub fn try_relief_sample(&self, x: f64, z: f64) -> Result<Option<ReliefSample>, RiverNetError> {
        relief::validate(&self.params)?;
        let point = self.macro_point(x, z);
        let Some(plate) = plate_query(point, self.seed, &self.params) else {
            return Ok(None);
        };
        let crust = crust_from_sample(point, &plate, self.seed, &self.params);
        let depth = self.relief_basin_depth(point, &plate)?;
        Ok(Some(relief::sample(
            &plate,
            &crust,
            depth,
            self.seed,
            &self.params,
        )))
    }

    /// Морфоклиматические веса T121/T122 в блочной точке. Только v1-dev.
    /// Для рельефа веса берутся с региональной решётки шагом `weight_step_l`
    /// и смешиваются билинейно: скачок R луча влаги на одном блоке не должен
    /// переключать форму рельефа ступенькой. Климат и биомные T/H этот фильтр
    /// не меняет; полное сглаживание климатических границ — T130–T133.
    pub fn morpho_weights(&self, x: f64, z: f64) -> Option<MorphoWeights> {
        morphoclimate::validate(&self.params).ok()?;
        let step = self.params.block.l_blocks * self.params.morpho.weight_step_l;
        let ix = (x / step).floor();
        let iz = (z / step).floor();
        let fx = x / step - ix;
        let fz = z / step - iz;
        let node = |dx: i64, dz: i64| -> Option<MorphoWeights> {
            let key = (ix as i64 + dx, iz as i64 + dz);
            if let Some(w) = self.morpho.get(key) {
                return Some(w);
            }
            let bx = key.0 as f64 * step;
            let bz = key.1 as f64 * step;
            let climate = self.climate_sample(bx, bz)?;
            let w = morphoclimate::weights(
                climate.temperature.t_bio,
                climate.precip.ratio,
                climate.surface.mountain,
                &self.params,
            );
            self.morpho.get_or_insert(key, || w);
            Some(w)
        };
        let w00 = node(0, 0)?;
        let w10 = node(1, 0)?;
        let w01 = node(0, 1)?;
        let w11 = node(1, 1)?;
        let mut out = MorphoWeights::default();
        for k in 0..crate::params::MORPHO_ZONES {
            let a = crate::math::lerp(w00.values[k], w10.values[k], fx);
            let b = crate::math::lerp(w01.values[k], w11.values[k], fx);
            out.values[k] = crate::math::lerp(a, b, fz);
        }
        let sum = out.sum();
        if sum > 0.0 {
            for value in out.values.iter_mut() {
                *value /= sum;
            }
        }
        Some(out)
    }

    /// Максимум глубины впадин по плите, чьи узлы могут попасть в носитель
    /// ядра. Фильтр полупространства Вороного отсекает заведомо далёкие сети.
    fn relief_basin_depth(&self, point: Vec2, nearest: &PlateSample) -> Result<f64, RiverNetError> {
        let step = self.params.drainage.step_l;
        let radius = 2.0 * step;
        let warp_bound = self.params.plates.warp_amplitude_l.abs() * crate::math::SQRT_2;
        let reach = radius + 4.0 * step + 2.0 * warp_bound;
        let mut depth: f64 = 0.0;
        for plate in plates_near(point, radius, self.seed, &self.params) {
            if plate.id != nearest.a.id {
                let delta = plate.seed_pos.sub(nearest.a.seed_pos);
                let separation = delta.len();
                if separation > 0.0 {
                    let beta = (nearest.q.dist2(plate.seed_pos)
                        - nearest.q.dist2(nearest.a.seed_pos))
                        / (2.0 * separation);
                    if beta > reach {
                        continue;
                    }
                }
            }
            let net = self.drainage_network(&plate)?;
            depth = depth.max(net.depression_depth_km(point));
        }
        Ok(depth)
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
        self.try_world_sample(x, z).ok().flatten()
    }

    /// Ошибки сети не маскируются под нулевую реку в диагностике. Игровой
    /// `field` остаётся infallible; проверяемые пути используют этот результат.
    pub fn try_world_sample(&self, x: f64, z: f64) -> Result<Option<WorldSample>, RiverNetError> {
        let Some(climate) = self.climate_sample(x, z) else {
            return Ok(None);
        };
        let c = continentalness(climate.surface.e_km, climate.surface.iota_l, &self.params);
        let is_ocean = c < self.params.climate.ocean_c_threshold;
        let class = classify(&climate, is_ocean, x, z, self.seed, &self.params);
        // v0 сохраняет старый путь; v1-dev подключает сеть T115 ниже.
        let point = self.macro_point(x, z);
        let surface = &climate.surface;
        let local = local_r(x, z, surface, self.seed, &self.params);
        // T103: до сети T110 быстрый и эталонный пути v1-dev используют
        // одинаковые локальные реки. Старые трассы не создают расхождение путей.
        let traced = if self.params.version.is_water_relief_dev() {
            0.0
        } else {
            traced_r(
                point,
                surface.e_km,
                self.seed,
                &self.params,
                &|q| self.surface_at(q),
                &self.rivers,
            )
        };
        let fields =
            self.surface_fields(&climate, class, is_ocean, x, z, local.max(traced), None)?;
        Ok(Some(WorldSample {
            climate,
            class: fields.class,
            vanilla: fields.vanilla,
            river: fields.river,
            hydrology: fields.hydrology,
            is_ocean: fields.is_ocean,
            pv: fields.pv,
            height: fields.height,
        }))
    }

    /// Полная сила реки вместе с трассированными руслами (слой `rivers`).
    pub fn river_sample(&self, x: f64, z: f64) -> f64 {
        let point = self.macro_point(x, z);
        let surface = self.surface_at(point);
        let local = local_r(x, z, &surface, self.seed, &self.params);
        let traced = if self.params.version.is_water_relief_dev() {
            0.0
        } else {
            traced_r(
                point,
                surface.e_km,
                self.seed,
                &self.params,
                &|q| self.surface_at(q),
                &self.rivers,
            )
        };
        let network = if self.params.version.is_water_relief_dev() {
            self.hydrology_at(x, z).map(|h| h.river).unwrap_or(0.0)
        } else {
            0.0
        };
        local.max(traced).max(network)
    }

    /// Сеть стока плиты с климатом, расходом и геометрией русел
    /// (T110–T115, только v1-dev). T115 переводит её в C/E/W и биомы;
    /// собственного PV и воды выше моря пока нет (T141/T143/T145).
    pub fn drainage_network(&self, plate: &Plate) -> Result<Arc<RiverNet>, RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        self.drainage.get_or_build(plate.id, || {
            RiverNet::build(
                plate,
                self.seed,
                &self.params,
                &|point| {
                    let surface = self.surface_at(point);
                    let outlet = if surface.e_km < 0.0 {
                        DrainageOutlet::Sea
                    } else if surface.sigma_l.abs() < self.params.drainage.step_l {
                        let k = surface.boundary_weights;
                        let min = self.params.drainage.outlet_min_weight;
                        if k[4] > min {
                            DrainageOutlet::Rift
                        } else if k[6] > min {
                            DrainageOutlet::Transform
                        } else {
                            DrainageOutlet::None
                        }
                    } else {
                        DrainageOutlet::None
                    };
                    DrainageInput {
                        route_km: surface.e_km,
                        outlet,
                    }
                },
                &|point| {
                    crate::climate::sample(point, self.seed, &self.params, &|q| self.surface_at(q))
                        .map(|sample| crate::runoff::RunoffClimate {
                            p_mm: sample.precip.p_mm,
                            pet_mm: sample.precip.pet_mm,
                            t_min_c: sample.temperature.t_min,
                        })
                },
            )
        })
    }

    /// Общий запрос русел T113 в блочной точке. Обходим консервативное окно
    /// плит в радиусе долина + меандр + кольцевой узел; порядок фиксирован id.
    /// Q соседних плит не складывается: межплитовая ширина постоянна (§5.5).
    pub fn drainage_channel_query(
        &self,
        x: f64,
        z: f64,
    ) -> Result<crate::channel::ChannelQuery, RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        let point = self.macro_point(x, z);
        let radius = crate::channel::max_query_radius_l(&self.params);
        let mut out = crate::channel::ChannelQuery::default();
        for plate in plates_near(point, radius, self.seed, &self.params) {
            let net = self.drainage_network(&plate)?;
            out.merge(net.channels().query(point));
        }
        Ok(out)
    }

    /// Сырой общий запрос русел/озёр для диагностики T114.
    pub fn drainage_surface_query(
        &self,
        x: f64,
        z: f64,
    ) -> Result<(crate::channel::ChannelQuery, crate::lake::LakeQuery), RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        self.hydrology_queries(self.macro_point(x, z))
    }

    fn hydrology_queries(
        &self,
        point: Vec2,
    ) -> Result<(crate::channel::ChannelQuery, crate::lake::LakeQuery), RiverNetError> {
        let radius = crate::channel::max_query_radius_l(&self.params);
        let mut channels = crate::channel::ChannelQuery::default();
        let mut lakes = crate::lake::LakeQuery::default();
        // T115:консервативный фильтр полупространства Вороного до построения сети.
        // A plate owns nodes only on its side of the bisector; the ring adds
        // one grid step outside it. The margin also covers warp and the query
        // radius, so a cross-plate channel is not dropped at a seam.
        let nearest = crate::plates::plate_query(point, self.seed, &self.params);
        let warp_bound = self.params.plates.warp_amplitude_l.abs() * crate::math::SQRT_2;
        let reach = radius + 4.0 * self.params.drainage.step_l + 2.0 * warp_bound;
        for plate in plates_near(point, radius, self.seed, &self.params) {
            if let Some(n) = &nearest {
                if plate.id != n.a.id {
                    let delta = plate.seed_pos.sub(n.a.seed_pos);
                    let separation = delta.len();
                    if separation > 0.0 {
                        let beta = (n.q.dist2(plate.seed_pos) - n.q.dist2(n.a.seed_pos))
                            / (2.0 * separation);
                        if beta > reach {
                            continue;
                        }
                    }
                }
            }
            let net = self.drainage_network(&plate)?;
            channels.merge(net.channels().query(point));
            lakes.merge(net.lakes().query(point));
        }
        Ok((channels, lakes))
    }

    /// Эффективная гидрология T115 в блочной точке. Кэш точной колонки не
    /// квантует координаты: попадание/промах не меняет ответ. Инфраструктурная
    /// ёмкость не входит в хеш мира; климат разрешается на месте вызова.
    pub fn hydrology_at(&self, x: f64, z: f64) -> Result<HydrologySample, RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        hydrology::validate(&self.params)?;
        let climate = self
            .climate_sample(x, z)
            .ok_or(RiverNetError::InvalidInput { cell: (0, 0) })?;
        let geometry = self.cached_hydrology_queries(x, z)?;
        Ok(hydrology::resolve(
            geometry.channels,
            geometry.lakes,
            &climate,
            &self.params,
        ))
    }

    fn cached_hydrology_queries(&self, x: f64, z: f64) -> Result<HydrologySample, RiverNetError> {
        // Сначала проверяем версию/параметры, затем строим сети вне замков.
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        hydrology::validate(&self.params)?;
        let key = (x.to_bits() as i64, z.to_bits() as i64);
        if let Some(hit) = self.hydrology.get(key) {
            return Ok(*hit);
        }
        let (channels, lakes) = self.hydrology_queries(self.macro_point(x, z))?;
        // Климат зависит от пути (reference/fast), поэтому кэшируем только
        // геометрию. Хранение готового kind здесь смешало бы два пути.
        let geometry = HydrologySample {
            channels,
            lakes,
            networks_considered: channels.networks_considered,
            ..HydrologySample::default()
        };
        self.hydrology.get_or_insert(key, || Arc::new(geometry));
        Ok(geometry)
    }

    #[allow(clippy::too_many_arguments)]
    fn surface_fields(
        &self,
        climate: &ClimateSample,
        class: crate::classify::ClassSample,
        is_ocean: bool,
        x: f64,
        z: f64,
        local_river: f64,
        known: Option<(&PlateSample, &CrustSample)>,
    ) -> Result<SurfaceFields, RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            let vanilla =
                crate::vanilla::sample(climate, &class, x, z, self.seed, &self.params, local_river);
            let pv = crate::vanilla::peaks_valleys(vanilla.w);
            return Ok(SurfaceFields {
                class,
                vanilla,
                river: local_river,
                hydrology: HydrologySample::default(),
                is_ocean,
                pv,
                height: crate::height::HeightSample::default(),
            });
        }
        let geometry = self.cached_hydrology_queries(x, z)?;
        let water = hydrology::resolve(geometry.channels, geometry.lakes, climate, &self.params);
        // T122: потенциал A и морфоклиматические зоны переводятся в C/E/W до
        // гидрологии T115/T116, чтобы русла/болота/дельты сохранили приоритет.
        let point = self.macro_point(x, z);
        // Быстрый путь уже посчитал плиту и кору этой же точки тем же вызовом;
        // эталон передаёт `None` и считает их здесь.
        let (plate, crust) = match known {
            Some((plate, crust)) => (*plate, *crust),
            None => {
                let plate = plate_query(point, self.seed, &self.params)
                    .ok_or(RiverNetError::InvalidInput { cell: (0, 0) })?;
                let crust = crust_from_sample(point, &plate, self.seed, &self.params);
                (plate, crust)
            }
        };
        let depth = self.relief_basin_depth(point, &plate)?;
        let relief_sample = relief::sample(&plate, &crust, depth, self.seed, &self.params);
        let morpho = self
            .morpho_weights(x, z)
            .ok_or(RiverNetError::InvalidInput { cell: (0, 0) })?;
        let base = crate::vanilla::sample(climate, &class, x, z, self.seed, &self.params, 0.0);
        let (base, pv_relief) = crate::relief_fields::apply_with_pv(
            base,
            climate,
            &relief_sample,
            &morpho,
            Vec2::new(x, z),
            self.seed,
            &self.params,
        );
        let vanilla = hydrology::apply_fields(base, local_river, &water, &self.params);
        let class = hydrology::classify_surface(class, &water, climate, &self.params);
        let river = local_river.max(water.river);
        // I2-lite: долины режут русла/низкие озёра, а не каждая смена знака W.
        let pv = crate::math::lerp(pv_relief, -1.0, crate::math::clamp01(river));
        // Д-057: в долине реки прибавка макровысоты (научный режим) гаснет,
        // иначе русло на плато остаётся выше моря и в игре сухое.
        let e_km = climate.surface.e_km;
        let e_valley_km = crate::math::lerp(
            e_km,
            e_km.min(self.params.height.e_gain_base_km),
            crate::math::clamp01(river),
        );
        let hp = &self.params.height;
        let micro = if hp.micro_amp_blocks.iter().any(|amp| *amp > 0.0) {
            crate::noise::fbm2_unit(
                crate::seed::subseed(self.seed, crate::seed::SALT_HEIGHT_MICRO),
                x / hp.micro_lambda_blocks,
                z / hp.micro_lambda_blocks,
                1.0,
                2,
                0.5,
            )
        } else {
            0.0
        };
        let height = crate::height::with_water(
            crate::height::sample_micro(
                vanilla.c,
                vanilla.e,
                pv,
                e_valley_km,
                morpho.values[6],
                morpho.values[7],
                micro,
                1.0 - crate::math::clamp01(river),
                &self.params,
            ),
            &water,
            &self.params,
        );
        Ok(SurfaceFields {
            class,
            vanilla,
            river,
            hydrology: water,
            is_ocean: class.is_ocean,
            pv,
            height,
        })
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
        self.try_sample_reference(x, z).ok().flatten()
    }

    pub fn try_sample_reference(
        &self,
        x: f64,
        z: f64,
    ) -> Result<Option<FullSample>, RiverNetError> {
        Ok(self
            .try_world_sample(x, z)?
            .map(|world| self.full_from_world(x, z, world)))
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
        self.try_sample(x, z).ok().flatten()
    }

    /// Быстрый путь с явной ошибкой сети для песочницы и регрессий.
    pub fn try_sample(&self, x: f64, z: f64) -> Result<Option<FullSample>, RiverNetError> {
        let Some(plates) = self.plate_sample(x, z) else {
            return Ok(None);
        };
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
        let Some(pole) = crate::climate::pole_sample(point, self.seed, &self.params) else {
            return Ok(None);
        };
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
        let exact_ray = || {
            crate::climate::trace_ray(
                point,
                &self.params,
                &|q| self.surface_at(q),
                dir,
                &wind_fn,
                self.seed,
            )
        };
        // Д-059: в v1-dev луч берётся с решётки; эталон считает его из точки.
        let ray = if self.params.version.is_water_relief_dev()
            && self.params.boundaries.ray_lattice_l > 0.0
            && self.params.precip.ray_steps > 0
        {
            self.lattice_ray(point)
                .map(|node| node.resolve(surface.e_km, pole.phi_deg, &self.params))
                .unwrap_or_else(exact_ray)
        } else {
            exact_ray()
        };
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
        let local = local_r(x, z, &surface, self.seed, &self.params);
        let fields = self.surface_fields(
            &climate,
            class,
            is_ocean,
            x,
            z,
            local,
            Some((&plates, &crust)),
        )?;
        let world = WorldSample {
            climate,
            class: fields.class,
            vanilla: fields.vanilla,
            river: fields.river,
            hydrology: fields.hydrology,
            is_ocean: fields.is_ocean,
            pv: fields.pv,
            height: fields.height,
        };
        let mut full = self.full_from_world(x, z, world);
        full.plate = Some(plates);
        Ok(Some(full))
    }

    /// `WorldSample` быстрого пути — для слоёв песочницы с `--path fast`.
    pub fn world_sample_fast(&self, x: f64, z: f64) -> Option<WorldSample> {
        self.sample(x, z).map(|full| full.world)
    }

    /// T160: кэш точной колонки. Ключ — биты координат, без квантования;
    /// попадание/промах не меняет ответ. Ошибки/None не кэшируются.
    /// К7: ошибку сети стока не проглатываем — `field()` пишет её в лог.
    /// Диагностика Д-059: тот же мир с другой решёткой луча влаги быстрого
    /// пути. Эталонный путь и хеш параметров не меняются; для игры решётка
    /// берётся только из `Params::defaults`.
    pub fn with_ray_lattice(mut self, step_l: f64) -> Self {
        self.params.boundaries.ray_lattice_l = step_l;
        self.rays = Arc::new(crate::cache::ShardedCache::new(
            self.params.lattice.cache_shards.max(1),
            RAY_CACHE_CAPACITY,
        ));
        self.column = Arc::new(crate::cache::ShardedCache::new(
            self.params.lattice.cache_shards.max(1),
            COLUMN_CACHE_CAPACITY,
        ));
        self.ray_traces = Arc::new(std::sync::atomic::AtomicU64::new(0));
        self
    }

    /// Диагностика Д-059: сколько узлов решётки луча посчитано с создания
    /// генератора (каждый узел — один веер лучей, как одна колонка эталона).
    pub fn ray_nodes(&self) -> u64 {
        self.ray_traces.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Д-059: наветренная часть луча влаги в узле решётки (`step` — её шаг
    /// в L). Узел считает свой ветер и свой веер так же, как точка в
    /// `try_sample`; `None` — у узла нет полюсов (тогда колонка считает луч
    /// сама).
    fn ray_node(&self, ix: i64, iz: i64, step: f64) -> Option<crate::climate::RayNode> {
        if let Some(hit) = self.rays.get((ix, iz)) {
            return hit;
        }
        self.ray_traces
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let q = Vec2::new(ix as f64 * step, iz as f64 * step);
        let value = crate::climate::pole_sample(q, self.seed, &self.params).map(|pole| {
            let wind = crate::climate::wind(pole.phi_deg, pole.north, pole.east, &self.params);
            let dir = if wind.len() > 1e-9 {
                wind.normalized()
            } else {
                crate::climate::fallback_dir(&self.params)
            };
            let wind_fn = |p: Vec2| crate::climate::wind_at(p, self.seed, &self.params);
            crate::climate::trace_ray_node(
                q,
                &self.params,
                &|p| self.surface_at(p),
                dir,
                &wind_fn,
                self.seed,
            )
        });
        self.rays.get_or_insert((ix, iz), || value)
    }

    /// Д-059: наветренная часть луча влаги для быстрого пути — билинейная
    /// смесь четырёх узлов решётки. Узел зависит только от своих координат,
    /// поэтому ответ не зависит от порядка запросов и от вытеснения из кэша.
    fn lattice_ray(&self, point: Vec2) -> Option<crate::climate::RayNode> {
        let step = self.params.boundaries.ray_lattice_l;
        let gx = point.x / step;
        let gz = point.z / step;
        let ix = gx.floor() as i64;
        let iz = gz.floor() as i64;
        let nodes = [
            self.ray_node(ix, iz, step)?,
            self.ray_node(ix + 1, iz, step)?,
            self.ray_node(ix, iz + 1, step)?,
            self.ray_node(ix + 1, iz + 1, step)?,
        ];
        Some(crate::climate::RayNode::bilinear(
            &nodes,
            gx - ix as f64,
            gz - iz as f64,
        ))
    }

    fn cached_full(&self, x: f64, z: f64) -> Result<Option<Arc<FullSample>>, RiverNetError> {
        let key = (x.to_bits() as i64, z.to_bits() as i64);
        if let Some(hit) = self.column.get(key) {
            return Ok(Some(hit));
        }
        let Some(full) = self.try_sample(x, z)? else {
            return Ok(None);
        };
        Ok(Some(self.column.get_or_insert(key, || Arc::new(full))))
    }

    /// Одно ванильное поле в блочных координатах: то, что зовёт мод (9.4).
    /// Ошибка/нечисловое значение больше не тихие: первые FIELD_LOG_LIMIT
    /// случаев попадают в stderr, все случаи считаются field_error_log_count().
    pub fn field(&self, field: Field, x: f64, _y: f64, z: f64) -> f64 {
        let full = match self.cached_full(x, z) {
            Ok(Some(full)) => full,
            Ok(None) => {
                log_field_error(|| format!("поле {:?} в ({x}, {z}): сэмпл отсутствует", field));
                return 0.0;
            }
            Err(error) => {
                log_field_error(|| format!("поле {:?} в ({x}, {z}): {error}", field));
                return 0.0;
            }
        };
        let value = match field {
            Field::Continents => full.vanilla.c,
            Field::Erosion => full.vanilla.e,
            Field::Ridges => full.vanilla.w,
            Field::Temperature => full.vanilla.t,
            Field::Vegetation => full.vanilla.h,
            Field::Lava => full.lava,
            Field::Offset => {
                if self.params.version.is_water_relief_dev() {
                    full.world.height.offset
                } else {
                    // v0 пак не переопределяет offset; сохраняем старое
                    // диагностическое значение, не меняя замороженный путь.
                    full.climate.surface.e_km
                }
            }
            Field::Pv => full.world.pv,
            Field::Factor => full.world.height.factor,
            Field::Jaggedness => full.world.height.jaggedness,
        };
        if value.is_finite() {
            value
        } else {
            log_field_error(|| {
                format!("поле {:?} в ({x}, {z}): нечисловое значение {value}", field)
            });
            0.0
        }
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
    fn v1_dev_fast_and_reference_rivers_agree() {
        let generator = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        for k in 0..24 {
            let x = k as f64 * 173.0 - 1800.0;
            let z = k as f64 * -97.0 + 900.0;
            let reference = generator.sample_reference(x, z).unwrap();
            let fast = generator.sample(x, z).unwrap();
            assert_eq!(reference.world.river, fast.world.river);
            assert_eq!(reference.vanilla.w.to_bits(), fast.vanilla.w.to_bits());
            assert_eq!(reference.vanilla.e.to_bits(), fast.vanilla.e.to_bits());
        }
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

    /// Д-057: опорные точки тестов берутся из построенной сети. Середины
    /// отрезков обычных русел рядом с `near`: (d², x, z, nx, nz, ядро в блоках,
    /// расход). `n` — единичная нормаль к отрезку.
    #[allow(clippy::type_complexity)]
    fn river_candidates(
        g: &Generator,
        near: (f64, f64),
        extra_l: f64,
    ) -> Vec<(f64, f64, f64, f64, f64, f64, f64)> {
        let mut out = Vec::new();
        let point = g.macro_point(near.0, near.1);
        let radius = crate::channel::max_query_radius_l(g.params()) + extra_l;
        let shift = g.spawn_shift();
        let l = g.params().block.l_blocks;
        for plate in crate::plates::plates_near(point, radius, g.seed(), g.params()) {
            let Ok(net) = g.drainage_network(&plate) else {
                continue;
            };
            let channels = net.channels();
            for segment in channels.segments() {
                if channels.reaches()[segment.reach].kind != crate::channel::ReachKind::Channel {
                    continue;
                }
                let a = segment.a.point;
                let b = segment.b.point;
                let x = (0.5 * (a.x + b.x) - shift.x) * l;
                let z = (0.5 * (a.z + b.z) - shift.z) * l;
                let n = b.sub(a).normalized().perp();
                let dx = x - near.0;
                let dz = z - near.1;
                out.push((
                    dx * dx + dz * dz,
                    x,
                    z,
                    n.x,
                    n.z,
                    0.5 * (segment.a.core_l + segment.b.core_l) * l,
                    0.5 * (segment.a.discharge_mm_l2 + segment.b.discharge_mm_l2),
                ));
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    fn find_river_core(g: &Generator, near: (f64, f64)) -> (f64, f64) {
        for (_, x, z, ..) in river_candidates(g, near, 4.0).into_iter().take(4096) {
            let Some(s) = g.sample(x, z) else {
                continue;
            };
            if s.world.hydrology.channels.core > 0.9
                && s.world.river > 0.9
                && s.cell.biome.ends_with("river")
                && s.vanilla.w.abs() < 0.01
            {
                return (x, z);
            }
        }
        panic!("не найдена точка русла рядом с {near:?}");
    }

    fn find_gallery_forest(g: &Generator, near: (f64, f64)) -> (f64, f64) {
        let q_min = g.params().hydrology.gallery_q_min_mm_l2;
        for (_, x, z, nx, nz, core, q) in river_candidates(g, near, 8.0) {
            if q < q_min {
                continue;
            }
            for k in [2.5_f64, -2.5, 3.5, -3.5, 4.5, -4.5] {
                let px = x + nx * core * k;
                let pz = z + nz * core * k;
                let Some(s) = g.sample(px, pz) else {
                    continue;
                };
                let h = s.world.hydrology;
                if h.gallery >= 0.5 && h.gallery > s.world.river && s.cell.biome == "forest" {
                    return (px, pz);
                }
            }
        }
        panic!("не найден галерейный лес рядом с {near:?}");
    }

    fn find_deferred_lake(g: &Generator, near: (f64, f64)) -> (f64, f64) {
        let point = g.macro_point(near.0, near.1);
        let shift = g.spawn_shift();
        let l = g.params().block.l_blocks;
        let r1 = g.params().hydrology.r1_max_level_km;
        let mut centers = Vec::new();
        for plate in crate::plates::plates_near(point, 12.0, g.seed(), g.params()) {
            let Ok(net) = g.drainage_network(&plate) else {
                continue;
            };
            for lake in net.lakes().lakes() {
                if lake.kind.has_water() && lake.level_km.is_some_and(|level| level > r1) {
                    let x = (lake.center.x - shift.x) * l;
                    let z = (lake.center.z - shift.z) * l;
                    let dx = x - near.0;
                    let dz = z - near.1;
                    centers.push((dx * dx + dz * dz, x, z));
                }
            }
        }
        centers.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, x, z) in centers {
            let Ok(h) = g.hydrology_at(x, z) else {
                continue;
            };
            if h.kind == crate::hydrology::SurfaceKind::DeferredWater
                && h.water == 0.0
                && h.r2_deferred == 1.0
            {
                return (x, z);
            }
        }
        panic!("не найдено высокое озеро рядом с {near:?}");
    }

    /// T115: точку дельты ищём по построенной сети, а не по фиксированным
    /// координатам. Подбор E/PV, впадин и меандров сдвигает русла, но тест
    /// должен проверять связку «сеть → игровые поля», а не запомненный пиксель.
    fn delta_swamp_candidates(g: &Generator, near: (f64, f64)) -> Vec<(f64, f64, f64)> {
        let mut out = Vec::new();
        let point = g.macro_point(near.0, near.1);
        let radius = crate::channel::max_query_radius_l(g.params()) + 4.0;
        let shift = g.spawn_shift();
        let l = g.params().block.l_blocks;
        for plate in crate::plates::plates_near(point, radius, g.seed(), g.params()) {
            let Ok(net) = g.drainage_network(&plate) else {
                continue;
            };
            let channels = net.channels();
            for segment in channels.segments() {
                if channels.reaches()[segment.reach].kind != crate::channel::ReachKind::Delta {
                    continue;
                }
                for t in [0.1_f64, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9] {
                    let mx = segment.a.point.x + t * (segment.b.point.x - segment.a.point.x);
                    let mz = segment.a.point.z + t * (segment.b.point.z - segment.a.point.z);
                    let x = (mx - shift.x) * l;
                    let z = (mz - shift.z) * l;
                    let dx = x - near.0;
                    let dz = z - near.1;
                    out.push((dx * dx + dz * dz, x, z));
                }
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    fn delta_swamp_sample(g: &Generator, x: f64, z: f64) -> bool {
        // Д-057: болото дельты даёт сам веер; мелкие впадины под ним теперь
        // прорезаются, поэтому отдельное болото-озеро не требуется.
        let Ok((channels, _lakes)) = g.drainage_surface_query(x, z) else {
            return false;
        };
        if channels.delta <= 0.9 {
            return false;
        }
        let Some(sample) = g.sample(x, z) else {
            return false;
        };
        sample.world.hydrology.channels.delta > 0.9
            && sample.world.hydrology.swamp > 0.9
            && sample.cell.biome.ends_with("swamp")
            && sample.vanilla.e > 0.55
    }

    fn find_delta_swamp(g: &Generator, near: (f64, f64)) -> (f64, f64) {
        let candidates = delta_swamp_candidates(g, near);
        let offsets = [-50.0_f64, 0.0, 50.0];
        for (_, x, z) in candidates.into_iter().take(256) {
            for dx in offsets {
                for dz in offsets {
                    let px = x + dx;
                    let pz = z + dz;
                    if delta_swamp_sample(g, px, pz) {
                        return (px, pz);
                    }
                }
            }
        }
        // Резервный скан: если дельта ушла на соседнюю плиту, сегменты другой
        // сети всё равно видны через общий запрос drainage_surface_query.
        for ix in -32..=32 {
            for iz in -32..=32 {
                let px = near.0 + ix as f64 * 125.0;
                let pz = near.1 + iz as f64 * 125.0;
                if delta_swamp_sample(g, px, pz) {
                    return (px, pz);
                }
            }
        }
        panic!("не найдена точка дельты с болотом рядом с {near:?}");
    }

    #[test]
    fn t115_network_lake_delta_and_cross_plate_reach_the_game_fields() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        // Прежняя опорная точка дельты: ищем актуальную точку поблизости по сети.
        let delta_near = (-54524.133387731, 24107.233303404);
        let delta_point = find_delta_swamp(&g, delta_near);
        let points = [
            find_deferred_lake(&g, (-35098.064436977, 8022.221972733)),
            delta_point,
            find_river_core(&g, (-1536.535252886, -2374.660469732)),
        ];
        let cold = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        for (x, z) in points {
            let h = g.hydrology_at(x, z).unwrap();
            let fast = g.try_sample(x, z).unwrap().unwrap();
            assert_eq!(fast.world.hydrology.kind, h.kind);
            assert_eq!(
                g.sample(x, z).unwrap().vanilla.w.to_bits(),
                fast.vanilla.w.to_bits()
            );
        }
        let lake = g.hydrology_at(points[0].0, points[0].1).unwrap();
        assert_eq!(lake.kind, crate::hydrology::SurfaceKind::DeferredWater);
        assert_eq!(lake.water, 0.0);
        assert_eq!(lake.r2_deferred, 1.0);
        let delta = g.sample(points[1].0, points[1].1).unwrap();
        assert!(delta.world.hydrology.channels.delta > 0.9);
        assert!(delta.world.hydrology.swamp > 0.9);
        assert!(delta.cell.biome.ends_with("swamp"));
        assert!(delta.vanilla.e > 0.55);
        let cross = g.sample(points[2].0, points[2].1).unwrap();
        assert!(cross.world.hydrology.channels.core > 0.9);
        assert!(cross.cell.biome.ends_with("river"));
        assert!(cross.vanilla.w.abs() < 0.01);
        let reference = cold
            .try_sample_reference(points[2].0, points[2].1)
            .unwrap()
            .unwrap();
        assert_eq!(cross.world.river.to_bits(), reference.world.river.to_bits());
        assert_eq!(cross.vanilla.c.to_bits(), reference.vanilla.c.to_bits());
        assert_eq!(cross.vanilla.e.to_bits(), reference.vanilla.e.to_bits());
        assert_eq!(cross.vanilla.w.to_bits(), reference.vanilla.w.to_bits());
    }

    #[test]
    fn t116_gallery_forest_reaches_a_real_dry_corridor() {
        // Д-057: коридор ищется по сети рядом с прежней опорной точкой.
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let (x, z) = find_gallery_forest(&g, (-1850.723375787, -12623.990320330));
        let fast = g.sample(x, z).unwrap();
        let reference = g.sample_reference(x, z).unwrap();
        assert!(fast.world.hydrology.gallery >= 0.5);
        assert!(fast.world.hydrology.gallery > fast.world.river);
        assert_eq!(fast.cell.biome, "forest");
        assert!(fast.vanilla.h > 0.0);
        // Д-059: быстрый путь берёт наветренную часть луча влаги с решётки,
        // поэтому H у него близко к эталону, но не побитово. Побитовое
        // равенство двух путей проверяется с выключенной решёткой.
        assert!((fast.vanilla.h - reference.vanilla.h).abs() < 0.02);
        assert_eq!(fast.vanilla.t.to_bits(), reference.vanilla.t.to_bits());
        assert_eq!(fast.cell, reference.cell);
        let exact = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev)
            .unwrap()
            .with_ray_lattice(0.0)
            .sample(x, z)
            .unwrap();
        assert_eq!(exact.vanilla.h.to_bits(), reference.vanilla.h.to_bits());
        assert_eq!(exact.vanilla.t.to_bits(), reference.vanilla.t.to_bits());
        assert_eq!(exact.cell, reference.cell);
    }

    #[test]
    fn t122_former_zone_flip_seams_stay_continuous() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        // T122: луч влаги здесь переключал полуаридную/аридную зону на одном
        // блоке. Региональная решётка весов обязана убрать ступень E/W.
        for (x, z, dx, dz) in [(480.0, -352.0, 1.0, 0.0), (-16.0, 224.0, 0.0, 1.0)] {
            let a = g.sample(x, z).unwrap();
            let b = g.sample(x + dx, z + dz).unwrap();
            assert!((a.vanilla.e - b.vanilla.e).abs() < 0.02);
            assert!((a.vanilla.w - b.vanilla.w).abs() < 0.02);
            let wa = g.morpho_weights(x, z).unwrap();
            let wb = g.morpho_weights(x + dx, z + dz).unwrap();
            for k in 0..crate::params::MORPHO_ZONES {
                assert!((wa.values[k] - wb.values[k]).abs() < 0.01);
            }
            let r = g.sample_reference(x, z).unwrap();
            assert_eq!(a.vanilla.e.to_bits(), r.vanilla.e.to_bits());
            assert_eq!(a.vanilla.w.to_bits(), r.vanilla.w.to_bits());
        }
    }

    #[test]
    fn t122_morpho_weights_are_continuous_across_grid_boundary() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let step = g.params().block.l_blocks * g.params().morpho.weight_step_l;
        let eps = 1e-6;
        for z in [-333.0, 0.0, 271.0] {
            let a = g.morpho_weights(step - eps, z).unwrap();
            let b = g.morpho_weights(step + eps, z).unwrap();
            for k in 0..crate::params::MORPHO_ZONES {
                assert!((a.values[k] - b.values[k]).abs() < 1e-6);
            }
            assert!((a.sum() - 1.0).abs() < 1e-12);
            assert!((b.sum() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn t121_morpho_weights_are_dev_only_and_sum_to_one() {
        let v0 = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        assert!(v0.morpho_weights(123.0, -456.0).is_none());
        let dev = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let w = dev.morpho_weights(123.0, -456.0).expect("климат есть");
        assert!((w.sum() - 1.0).abs() < 1e-12);
        assert!(w.values.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn v0_world_has_no_dev_hydrology() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        let w = g.world_sample(123.0, -456.0).unwrap();
        assert_eq!(w.hydrology, crate::hydrology::HydrologySample::default());
        assert!(g.hydrology_at(123.0, -456.0).is_err());
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

#[cfg(test)]
mod t160_tests {
    use super::Generator;
    use crate::field::Field;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    #[test]
    fn column_cache_does_not_change_field_answers_or_order() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let fields = [
            Field::Continents,
            Field::Erosion,
            Field::Ridges,
            Field::Temperature,
            Field::Vegetation,
            Field::Pv,
            Field::Offset,
            Field::Factor,
            Field::Jaggedness,
        ];
        for k in 0..24 {
            let x = k as f64 * 131.0 - 1500.0;
            let z = k as f64 * -97.0 + 700.0;
            let full = g.sample(x, z).unwrap();
            let expected = |f: Field| match f {
                Field::Continents => full.vanilla.c,
                Field::Erosion => full.vanilla.e,
                Field::Ridges => full.vanilla.w,
                Field::Temperature => full.vanilla.t,
                Field::Vegetation => full.vanilla.h,
                Field::Pv => full.world.pv,
                Field::Offset => full.world.height.offset,
                Field::Factor => full.world.height.factor,
                Field::Jaggedness => full.world.height.jaggedness,
                Field::Lava => full.lava,
            };
            for f in fields {
                assert_eq!(g.field(f, x, 0.0, z).to_bits(), expected(f).to_bits());
            }
            for f in fields.into_iter().rev() {
                assert_eq!(g.field(f, x, 64.0, z).to_bits(), expected(f).to_bits());
            }
        }
    }
}

#[cfg(test)]
mod t144_tests {
    use super::Generator;
    use crate::field::Field;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    #[test]
    fn owned_height_fields_match_the_world_sample_on_both_paths() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        for k in 0..16 {
            let x = k as f64 * 211.0 - 1600.0;
            let z = k as f64 * -173.0 + 900.0;
            let fast = g.sample(x, z).unwrap();
            let reference = g.sample_reference(x, z).unwrap();
            let h = fast.world.height;
            assert!(h.surface_y.is_finite());
            assert!((g.params().height.min_y..=g.params().height.max_y).contains(&h.surface_y));
            assert_eq!(h.offset.to_bits(), reference.world.height.offset.to_bits());
            assert_eq!(h.factor.to_bits(), reference.world.height.factor.to_bits());
            assert_eq!(
                h.jaggedness.to_bits(),
                reference.world.height.jaggedness.to_bits()
            );
            assert_eq!(
                g.field(Field::Offset, x, 0.0, z).to_bits(),
                h.offset.to_bits()
            );
            assert_eq!(
                g.field(Field::Factor, x, 0.0, z).to_bits(),
                h.factor.to_bits()
            );
            assert_eq!(
                g.field(Field::Jaggedness, x, 0.0, z).to_bits(),
                h.jaggedness.to_bits()
            );
        }
        assert_eq!(Field::ALL.len(), 10);
        assert_eq!(Field::parse("factor"), Some(Field::Factor));
        assert_eq!(Field::parse("jaggedness"), Some(Field::Jaggedness));
    }
}

#[cfg(test)]
mod t141_tests {
    use super::*;

    fn nearest_river_core(g: &Generator, near: (f64, f64)) -> (f64, f64) {
        let point = g.macro_point(near.0, near.1);
        let radius = crate::channel::max_query_radius_l(g.params()) + 4.0;
        let shift = g.spawn_shift();
        let l = g.params().block.l_blocks;
        let mut out = Vec::new();
        for plate in crate::plates::plates_near(point, radius, g.seed(), g.params()) {
            let Ok(net) = g.drainage_network(&plate) else {
                continue;
            };
            for segment in net.channels().segments() {
                let x = (0.5 * (segment.a.point.x + segment.b.point.x) - shift.x) * l;
                let z = (0.5 * (segment.a.point.z + segment.b.point.z) - shift.z) * l;
                let dx = x - near.0;
                let dz = z - near.1;
                out.push((dx * dx + dz * dz, x, z));
            }
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, x, z) in out.into_iter().take(4096) {
            if g.sample(x, z)
                .is_some_and(|s| s.world.river > 0.9 && s.world.pv < -0.8)
            {
                return (x, z);
            }
        }
        panic!("не найдена точка русла рядом с {near:?}");
    }

    #[test]
    fn pv_field_is_exposed_and_rivers_cut_their_own_valley() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        // Д-057: точка русла ищется по сети рядом с прежней опорной точкой.
        let (x, z) = nearest_river_core(&g, (-1536.535252886, -2374.660469732));
        let s = g.sample(x, z).unwrap();
        assert!(s.world.river > 0.9);
        assert!(s.world.pv.is_finite() && (-1.0..=1.0).contains(&s.world.pv));
        assert!(s.world.pv < -0.8, "русло не прорезало PV: {}", s.world.pv);
        assert_eq!(
            g.field(Field::Pv, x, 0.0, z).to_bits(),
            s.world.pv.to_bits()
        );
        assert_eq!(Field::parse("pv"), Some(Field::Pv));
        assert_eq!(Field::ALL.len(), 10);
    }

    #[test]
    fn v0_pv_keeps_the_old_derivative_of_w() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        let s = g.sample(0.0, 0.0).unwrap();
        assert_eq!(
            s.world.pv.to_bits(),
            crate::vanilla::peaks_valleys(s.vanilla.w).to_bits()
        );
    }
}

#[cfg(test)]
mod field_log_tests {
    use super::{FIELD_LOG_LIMIT, field_error_log_count, log_field_error_with};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn field_error_log_counts_all_errors_but_prints_a_limited_prefix() {
        // Локальный счётчик: параллельные тесты не влияют на проверку.
        let counter = AtomicU64::new(0);
        for number in 0..FIELD_LOG_LIMIT + 5 {
            log_field_error_with(&counter, || format!("тестовая ошибка {number}"));
        }
        assert_eq!(counter.load(Ordering::Relaxed), FIELD_LOG_LIMIT + 5);
        // Публичный счётчик доступен игре/CI и монотонно растёт.
        let _ = field_error_log_count();
    }
}

/// Д-059: решётка луча влаги быстрого пути.
#[cfg(test)]
mod ray_lattice_tests {
    use super::Generator;
    use crate::math::Vec2;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    fn points(n: usize) -> Vec<(f64, f64)> {
        (0..n)
            .map(|k| {
                (
                    k as f64 * 173.0 - 4100.5,
                    (k * k % 97) as f64 * 83.0 - 3900.5,
                )
            })
            .collect()
    }

    /// Узел без последнего шага плюс `resolve` в его же точке дают луч
    /// эталона — и на суше, и на море.
    #[test]
    fn node_reproduces_the_reference_ray_at_its_own_point() {
        for mode in Mode::ALL {
            let g = Generator::new(42, mode, GenVersion::V1Dev).unwrap();
            let (mut land, mut sea) = (0, 0);
            for (x, z) in points(60) {
                let point = g.macro_point(x, z);
                let pole = crate::climate::pole_sample(point, g.seed, &g.params).unwrap();
                let wind = crate::climate::wind(pole.phi_deg, pole.north, pole.east, &g.params);
                let dir = wind.normalized();
                let wind_fn = |p: Vec2| crate::climate::wind_at(p, g.seed, &g.params);
                let field = |p: Vec2| g.surface_at(p);
                let exact =
                    crate::climate::trace_ray(point, &g.params, &field, dir, &wind_fn, g.seed);
                let node =
                    crate::climate::trace_ray_node(point, &g.params, &field, dir, &wind_fn, g.seed);
                let e_km = g.surface_at(point).e_km;
                if e_km < 0.0 {
                    sea += 1;
                } else {
                    land += 1;
                }
                let got = node.resolve(e_km, pole.phi_deg, &g.params);
                assert!((got.mu - exact.mu).abs() < 1e-12, "μ в ({x}, {z})");
                assert!(
                    (got.cont_ray - exact.cont_ray).abs() < 1e-12,
                    "континентальность в ({x}, {z})"
                );
                assert!(
                    (got.slope - exact.slope).abs() < 1e-12,
                    "уклон в ({x}, {z})"
                );
            }
            assert!(land > 5 && sea > 5, "суша {land}, море {sea}");
        }
    }

    /// Ответ быстрого пути не зависит от порядка запросов: какие узлы
    /// считать, решают только значения узлов.
    #[test]
    fn fast_path_does_not_depend_on_query_order() {
        let pts = points(40);
        let forward = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let backward = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let a: Vec<_> = pts
            .iter()
            .map(|(x, z)| forward.sample(*x, *z).unwrap())
            .collect();
        let mut b: Vec<_> = pts
            .iter()
            .rev()
            .map(|(x, z)| backward.sample(*x, *z).unwrap())
            .collect();
        b.reverse();
        assert!(forward.ray_nodes() > 0);
        for (a, b) in a.iter().zip(&b) {
            assert_eq!(a.climate.precip.mu.to_bits(), b.climate.precip.mu.to_bits());
            assert_eq!(
                a.climate.precip.p_mm.to_bits(),
                b.climate.precip.p_mm.to_bits()
            );
            assert_eq!(a.vanilla.t.to_bits(), b.vanilla.t.to_bits());
            assert_eq!(a.vanilla.h.to_bits(), b.vanilla.h.to_bits());
            assert_eq!(a.zone, b.zone);
        }
    }

    /// v0 заморожен: решётка в нём выключена и ни один узел не считается.
    #[test]
    fn frozen_v0_never_uses_the_lattice() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V0).unwrap();
        assert_eq!(g.params.boundaries.ray_lattice_l, 0.0);
        for (x, z) in points(8) {
            let fast = g.sample(x, z).unwrap();
            let reference = g.sample_reference(x, z).unwrap();
            assert_eq!(
                fast.climate.precip.mu.to_bits(),
                reference.climate.precip.mu.to_bits()
            );
        }
        assert_eq!(g.ray_nodes(), 0);
    }

    /// Выключенная решётка возвращает быстрый путь v1-dev к лучу из колонки.
    #[test]
    fn disabled_lattice_matches_the_reference_bit_for_bit() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev)
            .unwrap()
            .with_ray_lattice(0.0);
        for (x, z) in points(8) {
            let fast = g.sample(x, z).unwrap();
            let reference = g.sample_reference(x, z).unwrap();
            assert_eq!(
                fast.climate.precip.p_mm.to_bits(),
                reference.climate.precip.p_mm.to_bits()
            );
        }
        assert_eq!(g.ray_nodes(), 0);
    }

    /// Решётка близка к эталону. На 2304 точках (`litho-sandbox paths`) зона
    /// совпадает в 0.973 точек; здесь выборка в 160 точек, поэтому порог
    /// 0.95 — он ловит поломку, а не меряет качество. Расхождения лежат на
    /// ступеньках μ эталона (луч с шагом 0.25 L пересекает берег).
    #[test]
    fn lattice_stays_close_to_the_reference() {
        for mode in Mode::ALL {
            let g = Generator::new(42, mode, GenVersion::V1Dev).unwrap();
            let pts = points(160);
            let mut same = 0_usize;
            let mut errors = Vec::with_capacity(pts.len());
            for (x, z) in &pts {
                let reference = g.sample_reference(*x, *z).unwrap();
                let fast = g.sample(*x, *z).unwrap();
                same += usize::from(reference.zone == fast.zone);
                errors.push((reference.climate.precip.mu - fast.climate.precip.mu).abs());
                // C/E/W от луча не зависят и остаются побитовыми.
                assert_eq!(reference.vanilla.c.to_bits(), fast.vanilla.c.to_bits());
                assert_eq!(reference.vanilla.e.to_bits(), fast.vanilla.e.to_bits());
                assert_eq!(reference.vanilla.w.to_bits(), fast.vanilla.w.to_bits());
            }
            errors.sort_by(f64::total_cmp);
            let share = same as f64 / pts.len() as f64;
            assert!(share >= 0.95, "{mode:?}: зона совпала в {share}");
            // На 2304 точках среднее |Δμ| 0.005, p95 0.025.
            assert!(
                errors[errors.len() / 2] < 0.01,
                "{mode:?}: медиана |Δμ| {}",
                errors[errors.len() / 2]
            );
        }
    }
}
