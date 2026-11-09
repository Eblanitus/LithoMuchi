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
use crate::drainage::{DrainageInput, DrainageOutlet, RiverNet, RiverNetCache, RiverNetError};
use crate::field::{Field, FullSample, to_f32};
use crate::hydrology::{self, HydrologySample};
use crate::lattice::{self, Nodes};
use crate::math::Vec2;
use crate::mode::Mode;
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
}

#[derive(Clone, Debug)]
pub struct Generator {
    seed: u64,
    params: Params,
    rivers: Arc<RiverCache>,
    hydrology: Arc<crate::cache::ShardedCache<(i64, i64), Arc<HydrologySample>>>,
    drainage: Arc<RiverNetCache>,
    coarse: Arc<CoarseTiles>,
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
        // T54: точка появления ищется один раз на мир, только по сиду и версии.
        let shift = spawn::find_shift(world_seed, &params);
        let shards = params.lattice.cache_shards.max(1);
        Ok(Self {
            seed: world_seed,
            params,
            rivers: Arc::new(RiverCache::new()),
            hydrology: Arc::new(crate::cache::ShardedCache::new(shards, 4096)),
            drainage: Arc::new(RiverNetCache::new(
                shards,
                crate::drainage::NET_CACHE_CAPACITY,
            )),
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
        let (class, vanilla, river, hydrology, is_ocean) =
            self.surface_fields(&climate, class, is_ocean, x, z, local.max(traced))?;
        Ok(Some(WorldSample {
            climate,
            class,
            vanilla,
            river,
            hydrology,
            is_ocean,
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

    fn surface_fields(
        &self,
        climate: &ClimateSample,
        class: crate::classify::ClassSample,
        is_ocean: bool,
        x: f64,
        z: f64,
        local_river: f64,
    ) -> Result<
        (
            crate::classify::ClassSample,
            VanillaSample,
            f64,
            HydrologySample,
            bool,
        ),
        RiverNetError,
    > {
        if !self.params.version.is_water_relief_dev() {
            let vanilla =
                crate::vanilla::sample(climate, &class, x, z, self.seed, &self.params, local_river);
            return Ok((
                class,
                vanilla,
                local_river,
                HydrologySample::default(),
                is_ocean,
            ));
        }
        let geometry = self.cached_hydrology_queries(x, z)?;
        let water = hydrology::resolve(geometry.channels, geometry.lakes, climate, &self.params);
        let base = crate::vanilla::sample(climate, &class, x, z, self.seed, &self.params, 0.0);
        let vanilla = hydrology::apply_fields(base, local_river, &water, &self.params);
        let class = hydrology::classify_surface(class, &water, climate, &self.params);
        let river = local_river.max(water.river);
        Ok((class, vanilla, river, water, class.is_ocean))
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
        let local = local_r(x, z, &surface, self.seed, &self.params);
        let (class, vanilla, river, hydrology, is_ocean) =
            self.surface_fields(&climate, class, is_ocean, x, z, local)?;
        let world = WorldSample {
            climate,
            class,
            vanilla,
            river,
            hydrology,
            is_ocean,
        };
        let mut full = self.full_from_world(x, z, world);
        full.plate = Some(plates);
        Ok(Some(full))
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

    #[test]
    fn t115_network_lake_delta_and_cross_plate_reach_the_game_fields() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let points = [
            (-36494.287478, 4844.677510),
            (-47696.263594, 16684.684891),
            (1155.684324, -1143.037898),
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
        let (x, z) = (178.059145, 413.141280);
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let fast = g.sample(x, z).unwrap();
        let reference = g.sample_reference(x, z).unwrap();
        assert!(fast.world.hydrology.gallery >= 0.5);
        assert!(fast.world.hydrology.gallery > fast.world.river);
        assert_eq!(fast.cell.biome, "forest");
        assert!(fast.vanilla.h > 0.0);
        assert_eq!(fast.vanilla.h.to_bits(), reference.vanilla.h.to_bits());
        assert_eq!(fast.vanilla.t.to_bits(), reference.vanilla.t.to_bits());
        assert_eq!(fast.cell, reference.cell);
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
