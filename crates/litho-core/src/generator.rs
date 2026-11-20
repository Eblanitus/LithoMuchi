//! Точка входа в ядро (эскиз публичного API 9.4).
//!
//! Здесь собраны примитивы, плиты, кора, климат, классификация и пять
//! ванильных параметров. Есть эталонный путь (`sample_reference`) и быстрый
//! путь через грубую решётку с кэшем тайлов (`sample`), плюс пять полей для
//! игры (`field`). Песочница и будущий мод вызывают один и тот же `Generator`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cache::{CACHE_CAPACITY, ShardedCache};
use crate::classify::{ClassSample, classify};
use crate::climate::{ClimateSample, PrecipSample, SurfaceSample, TemperatureSample, dryness};
use crate::crust::{CrustSample, crust_from_sample};
use crate::drainage::{
    DrainageInput, DrainageOutlet, PlateCache, PlateRouting, PlateTopo, RiverNet, RiverNetCache,
    RiverNetError,
};
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
/// Д-098: ячейки лавы 64×64; ёмкость на ответ не влияет.
const LAVA_CELL_CACHE_CAPACITY: usize = 4096;
/// Д-059: кэш лучей влаги в узлах решётки быстрого пути. Узел — два десятка
/// чисел; при шаге 32 блока ёмкости хватает на область 4096×4096 блоков.
/// В хеш мира не входит: вытеснение меняет только скорость, не ответ.
const RAY_CACHE_CAPACITY: usize = 16384;
/// К2/К7: `field()` раньше молча подменял ошибку и нечисловое значение нулём.
/// Д-061: сторона тайла списка «плиты с узлами рядом», в клетках решётки
/// стока. Инфраструктура: на ответ не влияет.
const NET_TILE_CELLS: i64 = 2;
/// Д-061: запас окна поверх радиуса запроса, в клетках: одна клетка на
/// кольцо сети и две сверху.
const NET_OWNER_MARGIN_CELLS: i64 = 3;
const NET_OWNER_CACHE_CAPACITY: usize = 8192;
/// T79: сторона блока «владелец узла решётки стока», в клетках. Окно списка
/// плит (Д-061) читает владельцев из блоков, а не считает каждый раз заново:
/// редкая колонка дальнего рельефа попадает в свой тайл списка, и без блоков
/// каждая платила за 28 × 28 запросов плиты. Инфраструктура: на ответ не
/// влияет.
const NET_OWNER_BLOCK_CELLS: i64 = 16;
const NET_OWNER_BLOCK_CACHE_CAPACITY: usize = 2048;

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

/// Постройка сети стока — самое долгое одиночное действие ядра (сотни мс).
/// Первые сети и затем каждая шестнадцатая попадают в stderr: по логу игры
/// видно, чем занято создание мира. На ответ не влияет.
const NET_LOG_FIRST: u64 = 12;
/// Дальше в лог идёт каждая сеть с номером, кратным 2^NET_LOG_EVERY_LOG2.
const NET_LOG_EVERY_LOG2: u32 = 4;
static NET_BUILDS: AtomicU64 = AtomicU64::new(0);

/// Сколько сетей стока построено в процессе (все миры). Диагностика.
pub fn net_build_count() -> u64 {
    NET_BUILDS.load(Ordering::Relaxed)
}

fn log_net_build(
    plate_id: u64,
    built: &Result<RiverNet, RiverNetError>,
    elapsed: std::time::Duration,
) {
    let number = NET_BUILDS.fetch_add(1, Ordering::Relaxed) + 1;
    let periodic = number.trailing_zeros() >= NET_LOG_EVERY_LOG2;
    if number > NET_LOG_FIRST && !periodic && built.is_ok() {
        return;
    }
    let ms = elapsed.as_secs_f64() * 1e3;
    match built {
        Ok(net) => eprintln!(
            "litho-core: сеть рек №{number}, плита {plate_id}: {} узлов за {ms:.0} мс",
            net.nodes().len()
        ),
        Err(error) => {
            eprintln!(
                "litho-core: сеть рек №{number}, плита {plate_id}: ошибка {error} за {ms:.0} мс"
            )
        }
    }
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
        eprintln!("litho-core: дальше сообщения field() подавляются");
    }
}

/// Узлов климатической решётки стока в памяти. На плиту приходится около
/// четырёхсот; ёмкость на ответ не влияет.
const RUNOFF_CLIMATE_CACHE_CAPACITY: usize = 65536;
type RunoffClimateCache = ShardedCache<(u64, u64), Option<crate::runoff::RunoffClimate>>;
/// Сколько топографий соседних плит строится одновременно.
const TOPO_PREFETCH_THREADS: usize = 8;

fn worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

type CoarseTiles = ShardedCache<(i64, i64), Arc<Nodes<COARSE_N>>>;
type LastCoarse = Option<(u64, u64, u64, i64, i64, Arc<Nodes<COARSE_N>>)>;
/// Д-061: тайл → плиты, чьи сети рек могут иметь узел рядом с запросом.
type NetOwners = ShardedCache<(i64, i64), Arc<Vec<u64>>>;
/// T79: блок решётки стока → владелец каждого узла блока.
type NetOwnerBlocks = ShardedCache<(i64, i64), Arc<Vec<Option<u64>>>>;

/// T79: недавние тайлы решётки высот на поток, прямое отображение.
const RAY_TILE_SLOTS: usize = 8;
type RayTileSlot = Option<(u64, i64, i64, Arc<crate::ray_field::RayFieldTile>)>;
/// Номер генератора для потоковой памяти тайлов: два мира в одном процессе
/// не должны читать тайлы друг друга (Р14).
static RAY_FIELD_IDS: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static RAY_TILES: RefCell<[RayTileSlot; RAY_TILE_SLOTS]> =
        const { RefCell::new([const { None }; RAY_TILE_SLOTS]) };
    /// «Последний тайл» на поток: соседние запросы не трогают замки (6.6).
    /// Ключ включает сид, **хеш параметров** и масштаб (Р14, Д-128): мирам с
    /// одним сидом и разными параметрами (`v1-dev` и `v1-dev-r2` в одном
    /// процессе) нельзя читать тайлы друг друга. Это страховка, а не причина
    /// паритета Ф4: `coarse_values` в поля не входит (только диагностика
    /// `coarse_probe`), а расхождение C/E/W шло из гидрологии (Д-127).
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

/// Тело водоёма в колонке (§6.8): что именно стоит в точке — река, озеро,
/// болото и т. п. Расширяет `SurfaceKind` классами озёр и точечными типами,
/// которые добавляются на этапах ВО3–ВО8.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Body {
    #[default]
    None,
    River,
    Lake,
    InlandSea,
    Swamp,
    DryBasin,
}

impl Body {
    /// Бит 12–17 в `info_code()` (§6.8).
    pub const fn code(self) -> u32 {
        match self {
            Self::None => 0,
            Self::River => 1,
            Self::Lake => 2,
            Self::InlandSea => 3,
            Self::Swamp => 4,
            Self::DryBasin => 5,
        }
    }
}

/// R2 (Ф6-В1, PLAN-WATER-FABRIC §5): вода колонки для заливки в игре.
/// `level` — верхний блок воды + 1 (вода стоит на `level − 1`), `bed` — дно
/// под зеркалом. `level = i32::MIN` значит «воды выше моря нет».
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaterColumn {
    pub level: i32,
    pub bed: i32,
    /// Поверхность колонки после выреза (дно под водой, §4 инварианта).
    pub surface: i32,
    pub kind: crate::hydrology::SurfaceKind,
    /// ВО2-2 (§6.8): тело водоёма.
    pub body: Body,
    /// ВО2-2 (§6.8): класс потока (спокойно, перекат, порог, каскад,
    /// водопад, котёл).
    pub flow: crate::water_profile::FlowClass,
    /// ВО2-2 (§7.5): код материала дна (ставит Java).
    pub bed_mat: u8,
    /// ВО2-2 (§7.5): код материала берега (ставит Java).
    pub shore_mat: u8,
}

impl WaterColumn {
    /// Метка «воды выше моря нет»: так же, как `i32::MIN` в соглашении FFI.
    pub const NONE: i32 = i32::MIN;

    /// Вода выше моря (R2). Русло у моря отдаётся с уровнем 63 (вода до
    /// y = 62) и сюда не входит: см. `has_sea_river`.
    pub const fn has_water(self) -> bool {
        self.level != Self::NONE && self.level > crate::height::SEA_LEVEL as i32
    }

    /// Русло у моря: вода до y = 62, прорезается Java по настоящим блокам.
    pub const fn has_sea_river(self) -> bool {
        self.level == crate::height::SEA_LEVEL as i32
    }

    /// Код вида воды для FFI и Java (Ф6-В2): без строк на каждый блок.
    pub const fn kind_code(self) -> u8 {
        match self.kind {
            crate::hydrology::SurfaceKind::Land => 0,
            crate::hydrology::SurfaceKind::River => 1,
            crate::hydrology::SurfaceKind::Lake => 2,
            crate::hydrology::SurfaceKind::InlandSea => 3,
            crate::hydrology::SurfaceKind::Swamp => 4,
            crate::hydrology::SurfaceKind::DryBasin => 5,
            crate::hydrology::SurfaceKind::DeferredWater => 6,
        }
    }

    /// ВО2-2 (§6.8): упакованный код для FFI ABI 3 — биты 0–3 `flow`,
    /// 4–7 `bed_mat`, 8–11 `shore_mat`, 12–17 `body`.
    pub const fn info_code(self) -> u32 {
        (self.flow.code() & 0xF)
            | ((self.bed_mat as u32 & 0xF) << 4)
            | ((self.shore_mat as u32 & 0xF) << 8)
            | ((self.body.code() as u32 & 0x3F) << 12)
    }
}

impl Default for WaterColumn {
    fn default() -> Self {
        Self {
            level: Self::NONE,
            bed: Self::NONE,
            surface: Self::NONE,
            kind: crate::hydrology::SurfaceKind::Land,
            body: Body::None,
            flow: crate::water_profile::FlowClass::Calm,
            bed_mat: 0,
            shore_mat: 0,
        }
    }
}

/// ВО2-2 (§6.8): `SurfaceKind` → `Body`; пока отражает существующие виды,
/// точечные типы озёр добавляются на ВО4.
fn body_from_kind(kind: crate::hydrology::SurfaceKind) -> Body {
    match kind {
        crate::hydrology::SurfaceKind::Land => Body::None,
        crate::hydrology::SurfaceKind::River => Body::River,
        crate::hydrology::SurfaceKind::Lake => Body::Lake,
        crate::hydrology::SurfaceKind::InlandSea => Body::InlandSea,
        crate::hydrology::SurfaceKind::Swamp => Body::Swamp,
        crate::hydrology::SurfaceKind::DryBasin => Body::DryBasin,
        crate::hydrology::SurfaceKind::DeferredWater => Body::None,
    }
}

/// ВО0-2 (PLAN-WATER-UPDATE §6.1): поверхность без воды `S0`.
///
/// `S0` — это рельеф, каким он был бы без рек и озёр: крупная форма
/// `macro_y` (без микробугров) плюс их вклад `micro_dy` отдельным слагаемым.
/// От `S0` в `v1-dev-w` считаются уровни воды, поэтому вырез рельефа в него
/// не входит и от гидрологии он не зависит.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct S0Sample {
    /// `S0_macro`: крупная форма поверхности в блоках, без микробугров.
    pub macro_y: f64,
    /// Вклад микробугров в блоках; ноль у версий без мелкого рельефа.
    pub micro_dy: f64,
}

impl S0Sample {
    /// `S0 = S0_macro + micro` — сухая поверхность колонки в блоках.
    pub const fn surface_y(&self) -> f64 {
        self.macro_y + self.micro_dy
    }
}

/// Результат перевода колонки в игровые поля (T115/T122/T141/T144).
struct SurfaceFields {
    /// К5 (Д-111): климат биомов — с вычетом температуры по высоте колонки.
    climate: ClimateSample,
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
    /// R2 (§4): уровни воды плиты от сухой поверхности. Отдельный кэш, потому
    /// что для их счёта нужна `S`, а `S` не зависит от уровней.
    r2_levels: Arc<PlateCache<PlateLevels>>,
    /// ВО1-1 (6.3): уровни `W` — профиль реки от `S0`. Кэш отдельный: в `W`
    /// уровни не читают `S` (иначе цикл «уровень → `S` → уровень»), а озёра
    /// пока живут по правилу R2 (6.4).
    w_levels: Arc<PlateCache<PlateLevels>>,
    /// R2 (§4): гидрология с уровнями воды. Кэш отдельный от сырого: сырой
    /// нужен счёту `S`, и в него уровни не попадают.
    hydrology_levels: Arc<crate::cache::ShardedCache<(i64, i64), Arc<HydrologySample>>>,
    /// Шаг 2 плана (T113): первая фаза сети плиты — топография без соседей.
    drainage_topo: Arc<PlateCache<PlateTopo>>,
    /// Шаг 2 плана (T113): вторая фаза — топография, связанная с соседями.
    drainage_routing: Arc<PlateCache<PlateRouting>>,
    /// Климат в узлах глобальной решётки стока. Чистая функция точки: приток
    /// от соседа читает те же узлы, что и его собственная сеть.
    runoff_climate: Arc<RunoffClimateCache>,
    /// Д-061: по тайлу — плиты, чьи сети могут иметь узел в радиусе запроса.
    net_owners: Arc<NetOwners>,
    /// T79: владельцы узлов решётки стока блоками, из них собирается список.
    net_owner_blocks: Arc<NetOwnerBlocks>,
    coarse: Arc<CoarseTiles>,
    /// T160: точная колонка `(x,z)` для повторных запросов полей.
    column: Arc<crate::cache::ShardedCache<(i64, i64), Arc<FullSample>>>,
    /// Д-059: лучи влаги в узлах решётки быстрого пути (только v1-dev).
    rays: Arc<crate::cache::ShardedCache<(i64, i64), Option<crate::climate::RayNode>>>,
    /// Д-059: счётчик посчитанных узлов, только для диагностики.
    ray_traces: Arc<std::sync::atomic::AtomicU64>,
    /// T79: высота, ветер и широта для луча влаги в узлах решётки.
    ray_field: Arc<ShardedCache<(i64, i64), Arc<crate::ray_field::RayFieldTile>>>,
    /// T79: ключ потоковой памяти тайлов этого генератора.
    ray_field_id: u64,
    /// T79: сколько узлов решётки высот посчитано, только для диагностики.
    ray_field_nodes: Arc<std::sync::atomic::AtomicU64>,
    /// Д-098: λ в центрах ячеек 64×64 и признак суши там же. Аквифер игры
    /// спрашивает одну ячейку сотни раз; кэш ответ не меняет.
    lava_cells: Arc<ShardedCache<(i64, i64), f64>>,
    lava_land: Arc<ShardedCache<(i64, i64), bool>>,
    /// К4: континентальность в центрах ячеек 4×4 блока. Пробы берега соседних
    /// колонок попадают в одни ячейки, чанк платит одну кору на ячейку.
    shore_c: Arc<ShardedCache<(i64, i64), f64>>,
    /// К4а/часть 2 (Д-110): ячейки кромки океана по квадратам `SHORE_BLOCK`.
    shore_edges: Arc<ShardedCache<(i64, i64), ShoreEdge>>,
    spawn_shift: Vec2,
}

/// К4а/часть 2 (Д-110): ячейки кромки океана одного квадрата `SHORE_BLOCK` —
/// номера ячеек 4×4 внутри квадрата.
type ShoreEdge = Arc<Vec<[u8; 2]>>;

/// R2 (PLAN-WATER-FABRIC §4): уровень звена стока по его сегментам.
///
/// `(start, end)` — зеркало в начале и в конце сегмента, блоки. Между ними
/// уровень интерполируется по `along`, поэтому ступень остаётся не больше
/// блока на блок русла. `None` — звено уже ушло на уровень моря.
///
/// Отдельный псевдоним, а не тип «в лоб»: clippy `type_complexity` валит
/// `-D warnings` на вложенном `Arc<Vec<Option<(f64, f64)>>>`.
type ReachLevels = Arc<Vec<Option<(f64, f64)>>>;
/// ВО1-1: классы звена по его сегментам.
type ReachProfiles = Arc<WReachProfile>;

/// R2 (PLAN-WATER-FABRIC §4): уровни воды плиты в блоках. Кэш плиты, потому
/// что в них входит сухая поверхность `S` в точках контура и берегов.
#[derive(Debug, Default)]
struct PlateLevels {
    /// По индексу `LakeNetwork::lakes()`: зеркало озера, блоки.
    lakes: Vec<Option<f64>>,
    /// Уровни русел по номеру звена: по сегментам звена, зеркало в начале и в
    /// конце. Уровень между ними интерполируется: так ступень остаётся не
    /// больше блока на блок русла. Считаются лениво, по одному звену: у плиты
    /// тысячи сегментов, и первая же колонка платила бы за все (замер В5).
    reaches: Mutex<HashMap<usize, ReachLevels>>,
    /// ВО1-1 (6.3, п. 7): профиль звена целиком — уровни и классы, только W.
    /// R2 этот словарь не читает.
    w_profiles: Mutex<HashMap<usize, Arc<WReachProfile>>>,
    /// Уровень входа звена (п. 3) — мемоизация отдельно от профиля: он
    /// считается в самом начале сборки звена и не меняется потом, а приёмник
    /// достраивает его же из соседних звеньев (п. 4). Без этой ячейки
    /// слияние двух притоков собирало бы их профили друг в друга без конца.
    w_entries: Mutex<HashMap<usize, Option<f64>>>,
    /// Уровень на конце звена ДО коррекции приёмника (п. 4): бегущий минимум
    /// уже посчитан, приёмник сюда не входит. Пишется один раз в начале
    /// сборки и никогда не переписывается, поэтому все читатели видят одно
    /// и то же значение независимо от порядка обхода. Без этой ячейки два
    /// слияния одной реки собирали бы профили друг друга заново до переполнения
    /// стека: коррекция приёмника (п. 4) смотрит вниз по течению и заново
    /// запрашивает звенья, которые ещё стоят на стеке вызова.
    w_ends: Mutex<HashMap<usize, f64>>,
    /// Родители каждого звена (`mouth_node == source_node`): по индексу
    /// звена. Строится один раз при сборке плиты, дальше только читается;
    /// без карты обход предков (`w_build_ancestors`) пересканировал бы
    /// весь список звеньев на каждый шаг.
    w_parents: Vec<Vec<usize>>,
    /// Звенья, которые сейчас стоят на стеке сборки (6.3, п. 3/8): цикл в
    /// сети стока встречается на круговых дорогах реки и без отметки
    /// завершился бы взаимной рекурсией `build → entry → parent → build`.
    w_in_progress: Mutex<HashMap<usize, std::thread::ThreadId>>,
}

/// Все родители звеньев сети (`mouth_node == source_node`), по индексу
/// звена. O(N) по словарю «узел-рот → звенья».
fn w_parents_of(net: &crate::drainage::RiverNet) -> Vec<Vec<usize>> {
    let channels = net.channels();
    let reaches = channels.reaches();
    let mut by_mouth: HashMap<usize, Vec<usize>> = HashMap::with_capacity(reaches.len());
    for (index, candidate) in reaches.iter().enumerate() {
        by_mouth
            .entry(candidate.mouth_node)
            .or_default()
            .push(index);
    }
    reaches
        .iter()
        .enumerate()
        .map(|(index, reach)| {
            by_mouth
                .get(&reach.source_node)
                .map(|list| list.iter().copied().filter(|&i| i != index).collect())
                .unwrap_or_default()
        })
        .collect()
}

/// ВО1-1: профиль одного звена — уровни и классы (6.3, п. 7).
#[derive(Debug)]
struct WReachProfile {
    levels: Vec<Option<(f64, f64)>>,
    classes: Vec<crate::water_profile::SegmentClass>,
}

/// §4, п. 1: точки контура озера берутся не реже чем через 2 блока.
const LAKE_RING_STEP_BLOCKS: f64 = 2.0;
/// Потолок числа точек кольца: чаще шага всё равно не нужно.
const LAKE_RING_MAX_SAMPLES: usize = 384;
/// §4, п. 2 и Д-090: запасной шаг вершин русла — 8 блоков.
const RIVER_LEVEL_STRIDE_BLOCKS: f64 = 8.0;
/// Д-090: сеть плиты с уровнями не дольше 200 мс; на уровни — не больше 120.
const RIVER_LEVEL_BUDGET_MS: u128 = 120;

/// К4: сторона ячейки, в центре которой берётся континентальность берега.
const SHORE_CELL: f64 = 4.0;
/// К4а/часть 2 (Д-110): сторона квадрата, по которому кэшируется кромка океана.
const SHORE_BLOCK: f64 = 32.0;

/// ВО1: точка `S0` с входами классов 5.2. Один и тот же путь во всех
/// пробах профиля `W` и в `s0_at`: `macro_y` совпадает побитово, расхождения
/// «уровни против мира» нет по построению.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct WProbe {
    /// `S0_macro` в блоках.
    macro_y: f64,
    /// Сырой единичный шум микробугров (не взвешенный).
    micro: f64,
    pv_relief: f64,
    base_c: f64,
    base_e: f64,
    e_km: f64,
    cold: f64,
    warm: f64,
    shore_dist: f64,
    /// Полоса E после макрорельефа — та же строка таблицы высот.
    erosion_e: f64,
    /// `t_c` (пояс P0–P6).
    t_c: f64,
    /// `R = PET/P`.
    aridity: f64,
    plateau: f64,
    /// Вес морфозоны 6 (холодные горы).
    cold_mountain_weight: f64,
}

impl Generator {
    /// ВО1: общая точка `S0` для профиля. `None` — версия без водного
    /// обновления или точка без климата/плиты.
    fn w_probe(&self, x: f64, z: f64) -> Option<WProbe> {
        if !self.params.version.is_water_update() {
            return None;
        }
        let climate = self.climate_sample(x, z)?;
        let point = self.macro_point(x, z);
        let plate = plate_query(point, self.seed, &self.params)?;
        let crust = crust_from_sample(point, &plate, self.seed, &self.params);
        let c = continentalness(climate.surface.e_km, climate.surface.iota_l, &self.params);
        let is_ocean = c < self.params.climate.ocean_c_threshold;
        let class = self.class_before_height(&climate, is_ocean, x, z);
        let depth = self.relief_basin_depth(point, &plate).ok()?;
        let crust_c = continentalness(crust.e_km, crust.iota_l, &self.params);
        let shore_dist = self.shore_distance_blocks(x, z, crust_c);
        let shore_fade = self.shore_fade_from(shore_dist);
        let relief_sample =
            relief::sample_near_shore(&plate, &crust, depth, shore_fade, self.seed, &self.params);
        let morpho = self.morpho_weights(x, z)?;
        // `local_river = 0`: в `S0` воды нет по определению.
        let base = crate::vanilla::sample(&climate, &class, x, z, self.seed, &self.params, 0.0);
        let (base, pv_relief) = crate::relief_fields::apply_with_pv(
            base,
            &climate,
            &relief_sample,
            &morpho,
            Vec2::new(x, z),
            self.seed,
            &self.params,
        );
        let hp = &self.params.height;
        // Тот же шум и та же длина волны, что в `surface_fields`; в `S0` он
        // не гасится рекой (`gate = 1`), а вычитается отдельным слагаемым.
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
        let shore_dist = if base.c <= self.params.climate.ocean_c_threshold {
            0.0
        } else {
            shore_dist
        };
        let cold = morpho.values[6];
        let warm = morpho.values[7];
        let macro_h = crate::height::sample_micro(
            base.c,
            base.e,
            pv_relief,
            climate.surface.e_km,
            cold,
            warm,
            0.0,
            0.0,
            shore_dist,
            &self.params,
        );
        Some(WProbe {
            macro_y: macro_h.surface_y,
            micro,
            pv_relief,
            base_c: base.c,
            base_e: base.e,
            e_km: climate.surface.e_km,
            cold,
            warm,
            shore_dist,
            erosion_e: base.e,
            t_c: climate.temperature.t_c,
            aridity: climate.precip.ratio,
            plateau: crust.plateau,
            cold_mountain_weight: morpho.values[6],
        })
    }
}

impl Generator {
    /// Создаётся один раз на мир. Пока дёшево; после T54 здесь появится поиск
    /// сдвига точки появления `p*` (около 256 эталонных расчётов).
    pub fn new(seed: i64, mode: Mode, version: GenVersion) -> Result<Self, GenError> {
        if version == GenVersion::V1 {
            return Err(GenError::UnsupportedVersion(version));
        }
        Self::with_params(seed, Params::defaults(version, mode))
    }

    /// T151: генератор с изменёнными параметрами — только для подбора в
    /// песочнице (`--set`). Мод и приёмка идут через [`Generator::new`]:
    /// мир с другими параметрами — другой мир с другим `params_hash`.
    #[doc(hidden)]
    pub fn with_params(seed: i64, mut params: Params) -> Result<Self, GenError> {
        let version = params.version;
        match version {
            GenVersion::V0 | GenVersion::V1Dev | GenVersion::V1DevR2 | GenVersion::V1DevW => {}
            GenVersion::V1 => return Err(GenError::UnsupportedVersion(version)),
        }
        // Ключ кэшей плит обязан соответствовать фактическим параметрам.
        params.config_key = params.params_hash();
        let world_seed = from_world_seed(seed);
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
            r2_levels: Arc::new(PlateCache::new(shards, crate::drainage::NET_CACHE_CAPACITY)),
            w_levels: Arc::new(PlateCache::new(shards, crate::drainage::NET_CACHE_CAPACITY)),
            hydrology_levels: Arc::new(crate::cache::ShardedCache::new(shards, 4096)),
            drainage_topo: Arc::new(PlateCache::new(shards, crate::drainage::NET_CACHE_CAPACITY)),
            drainage_routing: Arc::new(PlateCache::new(
                shards,
                crate::drainage::NET_CACHE_CAPACITY,
            )),
            runoff_climate: Arc::new(ShardedCache::new(shards, RUNOFF_CLIMATE_CACHE_CAPACITY)),
            net_owners: Arc::new(crate::cache::ShardedCache::new(
                shards,
                NET_OWNER_CACHE_CAPACITY,
            )),
            net_owner_blocks: Arc::new(ShardedCache::new(shards, NET_OWNER_BLOCK_CACHE_CAPACITY)),
            coarse: Arc::new(ShardedCache::new(shards, CACHE_CAPACITY)),
            column: Arc::new(crate::cache::ShardedCache::new(
                shards,
                COLUMN_CACHE_CAPACITY,
            )),
            rays: Arc::new(crate::cache::ShardedCache::new(shards, RAY_CACHE_CAPACITY)),
            ray_traces: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            ray_field: Arc::new(ShardedCache::new(
                shards,
                crate::ray_field::TILE_CACHE_CAPACITY,
            )),
            ray_field_id: RAY_FIELD_IDS.fetch_add(1, Ordering::Relaxed),
            ray_field_nodes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            lava_cells: Arc::new(ShardedCache::new(shards, LAVA_CELL_CACHE_CAPACITY)),
            lava_land: Arc::new(ShardedCache::new(shards, LAVA_CELL_CACHE_CAPACITY)),
            shore_c: Arc::new(ShardedCache::new(shards, LAVA_CELL_CACHE_CAPACITY)),
            shore_edges: Arc::new(ShardedCache::new(shards, LAVA_CELL_CACHE_CAPACITY)),
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
        let shore_fade = self.shore_relief_fade(x, z, &crust);
        Ok(Some(relief::sample_near_shore(
            &plate,
            &crust,
            depth,
            shore_fade,
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
        let owners = self.net_owners(point);
        let mut depth: f64 = 0.0;
        for plate in plates_near(point, radius, self.seed, &self.params) {
            if owners.binary_search(&plate.id).is_err() {
                continue;
            }
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

    /// Д-061: плиты, чьи сети стока могут иметь узел в радиусе общего запроса
    /// русел от любой точки тайла, по возрастанию id.
    ///
    /// Вклад сети в точку (русло, озеро, глубина впадины) идёт только от её
    /// узлов не дальше `max_query_radius_l`. Узел сети — узел плиты или сосед
    /// такого узла (кольцо). Значит, плита без своих узлов в окне «тайл +
    /// радиус + кольцо + запас» в ответ не входит. Прежний фильтр по границе
    /// Вороного брал запас на искажение домена 2·4√2 L и пропускал почти всех
    /// соседей: первая колонка мира строила 13 сетей.
    ///
    /// Список — чистая функция сида, параметров и тайла; кэш на ответ не влияет.
    fn net_owners(&self, point: Vec2) -> Arc<Vec<u64>> {
        let h = self.params.drainage.step_l;
        let side = h * NET_TILE_CELLS as f64;
        let tile = (
            (point.x / side).floor() as i64,
            (point.z / side).floor() as i64,
        );
        self.net_owners.get_or_insert(tile, || {
            let reach = ((crate::channel::max_query_radius_l(&self.params) / h).ceil() as i64)
                .saturating_add(NET_OWNER_MARGIN_CELLS);
            let first = |t: i64| t.saturating_mul(NET_TILE_CELLS).saturating_sub(reach);
            let last = |t: i64| {
                t.saturating_mul(NET_TILE_CELLS)
                    .saturating_add(NET_TILE_CELLS - 1)
                    .saturating_add(reach)
            };
            Arc::new(
                self.owners_in_window((first(tile.0), first(tile.1)), (last(tile.0), last(tile.1))),
            )
        })
    }

    /// T79: то же, что `drainage::owners_in_window`, но владельцы узлов
    /// читаются из блоков `NET_OWNER_BLOCK_CELLS`, посчитанных один раз.
    /// Ответ совпадает с прямым расчётом (тест ниже).
    fn owners_in_window(&self, lo: (i64, i64), hi: (i64, i64)) -> Vec<u64> {
        const B: i64 = NET_OWNER_BLOCK_CELLS;
        let mut out: Vec<u64> = Vec::new();
        let mut last = None;
        for bz in lo.1.div_euclid(B)..=hi.1.div_euclid(B) {
            for bx in lo.0.div_euclid(B)..=hi.0.div_euclid(B) {
                let block = self.net_owner_blocks.get_or_insert((bx, bz), || {
                    Arc::new(crate::drainage::owner_block(
                        bx,
                        bz,
                        B,
                        self.seed,
                        &self.params,
                    ))
                });
                let (x0, z0) = (bx.saturating_mul(B), bz.saturating_mul(B));
                for j in lo.1.max(z0)..=hi.1.min(z0.saturating_add(B - 1)) {
                    for i in lo.0.max(x0)..=hi.0.min(x0.saturating_add(B - 1)) {
                        let Some(id) = block[((j - z0) * B + (i - x0)) as usize] else {
                            continue;
                        };
                        if last != Some(id) {
                            last = Some(id);
                            if !out.contains(&id) {
                                out.push(id);
                            }
                        }
                    }
                }
            }
        }
        out.sort_unstable();
        out
    }

    /// Диагностика искажения домена (слой `jacobian`).
    pub fn jacobian(&self, x: f64, z: f64) -> f64 {
        warp_jacobian(self.macro_point(x, z), self.seed, &self.params)
    }

    /// Поверхность в произвольной macro-точке: нужно линии тока влаги (7.7).
    fn surface_at(&self, point: Vec2) -> SurfaceSample {
        match plate_query(point, self.seed, &self.params) {
            Some(sample) => self.surface_from_sample(point, &sample),
            None => SurfaceSample::default(),
        }
    }

    fn surface_from_sample(
        &self,
        point: Vec2,
        sample: &crate::plates::PlateSample,
    ) -> SurfaceSample {
        let crust = crust_from_sample(point, sample, self.seed, &self.params);
        SurfaceSample {
            e_km: crust.e_km,
            iota_l: crust.iota_l,
            mountain: crust.mountain,
            plateau: crust.plateau,
            boundary_weights: crust.boundary_weights,
            sigma_l: sample.d_ab_l,
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
        let class = self.class_before_height(&climate, is_ocean, x, z);
        // v0 сохраняет старый путь; v1-dev подключает сеть T115 ниже.
        let point = self.macro_point(x, z);
        let surface = &climate.surface;
        // ВО1-3 (§9): в `v1-dev-w` карандашных рек нет — воду ведёт профиль `W`.
        let local = if self.params.version.is_water_update() {
            0.0
        } else {
            local_r(x, z, surface, self.seed, &self.params)
        };
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
        // Эталонный путь: колонка с водой, как в игре (`apply_water = true`).
        let fields = self.surface_fields(
            &climate,
            class,
            is_ocean,
            x,
            z,
            local.max(traced),
            None,
            true,
        )?;
        Ok(Some(WorldSample {
            climate: fields.climate,
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
        let local = if self.params.version.is_water_update() {
            0.0
        } else {
            local_r(x, z, &surface, self.seed, &self.params)
        };
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
    ///
    /// Шаг 2 плана (T113): сеть собирается в три фазы. Топография плиты
    /// ([`PlateTopo`]) от соседей не зависит. Связанная топография
    /// ([`PlateRouting`]) читает у соседей уровни ухода воды и решает, где
    /// вода уходит через границу. Расход получает приток от соседних плит,
    /// прослеженный вверх по течению не дальше `drainage.handoff_max_hops`
    /// границ. Ни одна фаза не обращается к готовой сети другой плиты, поэтому
    /// рекурсии между кэшами нет.
    pub fn drainage_network(&self, plate: &Plate) -> Result<Arc<RiverNet>, RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        self.drainage.get_or_build(plate.id, || {
            let started = std::time::Instant::now();
            let built = self.build_drainage_network(plate);
            log_net_build(plate.id, &built, started.elapsed());
            built
        })
    }

    /// Первая фаза сети плиты: узлы, высоты, выходы и заливка по своим узлам.
    pub fn drainage_topo(&self, plate: &Plate) -> Result<Arc<PlateTopo>, RiverNetError> {
        self.drainage_topo_with(plate, worker_threads())
    }

    /// `threads` — потоки на высоты узлов одной топографии; на ответ не влияет.
    fn drainage_topo_with(
        &self,
        plate: &Plate,
        threads: usize,
    ) -> Result<Arc<PlateTopo>, RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        self.drainage_topo.get_or_build(plate.id, || {
            PlateTopo::build(
                plate,
                self.seed,
                &self.params,
                &|point, sample| {
                    // Тот же ответ, что у `surface_at(point)`, без второго
                    // запроса плиты.
                    let surface = sample.map_or_else(SurfaceSample::default, |sample| {
                        self.surface_from_sample(point, sample)
                    });
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
                    let l = self.params.block.l_blocks;
                    let route_km = if self.params.version.is_water_update() {
                        // ВО3-1 (§6.2): маршрут по S0 — S0_macro(узел) +
                        // τ·clamp(e_km, 0, 0.8) в блоках, затем в км стока.
                        let (bx, bz) = (point.x * l, point.z * l);
                        let s0_macro = self
                            .s0_at(bx, bz)
                            .map(|s| s.macro_y)
                            .unwrap_or_else(|| surface.e_km * self.params.height.water_km_to_blocks + crate::height::SEA_LEVEL as f64);
                        let tau = self.params.water.route_tau_blocks_per_km;
                        let route_y = s0_macro + tau * surface.e_km.clamp(0.0, 0.8);
                        (route_y - crate::height::SEA_LEVEL as f64) / self.params.height.water_km_to_blocks
                    } else {
                        surface.e_km
                    };
                    DrainageInput {
                        route_km,
                        outlet,
                    }
                },
                threads,
            )
        })
    }

    /// Топографии нескольких плит; недостающие строятся в несколько потоков.
    /// Порядок и число потоков на ответ не влияют: каждая топография —
    /// чистая функция сида, параметров и плиты.
    fn drainage_topos(&self, plates: &[Plate]) -> Result<Vec<Arc<PlateTopo>>, RiverNetError> {
        let missing: Vec<&Plate> = plates
            .iter()
            .filter(|plate| !self.drainage_topo.is_ready(plate.id))
            .collect();
        let workers = worker_threads();
        let threads = workers.min(TOPO_PREFETCH_THREADS).min(missing.len());
        // Потоки, оставшиеся свободными, делятся между топографиями.
        let inner = (workers / threads.max(1)).max(1);
        if threads >= 2 {
            let next = std::sync::atomic::AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for _ in 0..threads {
                    scope.spawn(|| {
                        loop {
                            let k = next.fetch_add(1, Ordering::Relaxed);
                            let Some(plate) = missing.get(k) else {
                                break;
                            };
                            // Ошибку вернёт обычный запрос ниже.
                            let _ = self.drainage_topo_with(plate, inner);
                        }
                    });
                }
            });
        }
        plates
            .iter()
            .map(|plate| self.drainage_topo(plate))
            .collect()
    }

    /// Вторая фаза: топография плиты, связанная с уровнями соседей.
    pub fn drainage_routing(&self, plate: &Plate) -> Result<Arc<PlateRouting>, RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        self.drainage_routing.get_or_build(plate.id, || {
            let topo = self.drainage_topo(plate)?;
            let around = self.drainage_topos(topo.neighbors())?;
            PlateRouting::build(&topo, &self.params, &|owner, cell| {
                let k = around.binary_search_by_key(&owner, |t| t.plate_id()).ok()?;
                around[k].escape_level_km(cell)
            })
        })
    }

    /// Климат стока в узле решётки; значение запоминается по координатам узла.
    fn runoff_climate_at(&self, point: Vec2) -> Option<crate::runoff::RunoffClimate> {
        self.runoff_climate
            .get_or_insert((point.x.to_bits(), point.z.to_bits()), || {
                crate::climate::sample(point, self.seed, &self.params, &|q| self.surface_at(q)).map(
                    |sample| crate::runoff::RunoffClimate {
                        p_mm: sample.precip.p_mm,
                        pet_mm: sample.precip.pet_mm,
                        t_min_c: sample.temperature.t_min,
                    },
                )
            })
    }

    /// Приток в свои узлы сети `routing` от соседних плит, мм·L²/год.
    /// `hops` — сколько ещё границ можно пройти вверх по течению; `wanted`
    /// ограничивает узлы, приток в которые нужен вызывающему.
    ///
    /// Узел своей плиты лежит в кольце соседа; расход соседа в этом узле и
    /// есть переданная вода. Сосед считается только в части, которая в эти
    /// узлы стекает, поэтому работа пропорциональна настоящему водосбору.
    fn handoff_inflow(
        &self,
        routing: &PlateRouting,
        hops: i64,
        wanted: Option<&[bool]>,
    ) -> Result<Vec<f64>, RiverNetError> {
        let mut inflow = vec![0.0; routing.nodes().len()];
        if hops <= 0 || routing.neighbors().is_empty() {
            return Ok(inflow);
        }
        let own = self.drainage_topo(routing.plate())?;
        let is_wanted = |cell: (i64, i64)| {
            routing.node_index(cell).is_some_and(|k| {
                let node = &routing.nodes()[k];
                node.in_plate && !node.outlet.is_outlet() && wanted.is_none_or(|mask| mask[k])
            })
        };
        // Топографии нужны только у соседей, чья суша касается нужных узлов:
        // для целой плиты это все её соседи, они уже построены; для
        // водосбора вложенного вызова — обычно никто.
        let candidates = if wanted.is_none() {
            routing.neighbors().to_vec()
        } else {
            own.neighbors_touching(&is_wanted)
        };
        let around = self.drainage_topos(&candidates)?;
        let sources: Vec<(&Plate, &Arc<PlateTopo>)> = candidates
            .iter()
            .zip(&around)
            .filter(|(_, topo)| topo.may_hand_to(&own, &is_wanted))
            .collect();
        // Вклад одного соседа: (свой узел, расход). Соседи независимы.
        let me = routing.plate_id();
        // Узлы-передачи сети соседа, лежащие в нужных узлах этой плиты, свои
        // узлы-приёмники и водосбор этих передач.
        type Catchment = (Vec<usize>, Vec<usize>, Vec<bool>);
        let catchment = |source: &PlateRouting| -> Result<Option<Catchment>, RiverNetError> {
            let mut targets = Vec::new();
            let mut receivers = Vec::new();
            for (k, node) in source.nodes().iter().enumerate() {
                if node.outlet != DrainageOutlet::Handoff {
                    continue;
                }
                let Some(here) = routing.node_index(node.cell) else {
                    continue;
                };
                if is_wanted(node.cell) {
                    targets.push(k);
                    receivers.push(here);
                }
            }
            if targets.is_empty() {
                return Ok(None);
            }
            let mask =
                crate::runoff::upstream_mask(source.nodes(), source.depressions(), &targets)?;
            // Кроме самих узлов-передач в маске должен быть хоть один узел.
            if mask.iter().filter(|m| **m).count() == targets.len() {
                return Ok(None);
            }
            Ok(Some((targets, receivers, mask)))
        };
        let contribution =
            |neighbor: &Plate, topo: &PlateTopo| -> Result<Vec<(usize, f64)>, RiverNetError> {
                // Д-094. Настоящей связанной сети соседа нужны топографии всех
                // его соседей. Чаще всего они не нужны: строим две сети соседа,
                // связанные только с этой плитой. В первой третьи плиты воду не
                // принимают совсем, во второй принимают на самом низком возможном
                // уровне — высоте своего узла. Настоящая сеть лежит между ними.
                // Если в первой сюда ничего не течёт, не течёт и в настоящей.
                // Если водосбор в обеих один и тот же, он такой же и в настоящей.
                // Иначе строим настоящую сеть.
                let closed = PlateRouting::build(topo, &self.params, &|owner, cell| {
                    (owner == me).then(|| own.escape_level_km(cell)).flatten()
                })?;
                let Some(found) = catchment(&closed)? else {
                    return Ok(Vec::new());
                };
                let settled = topo.neighbors().iter().all(|plate| plate.id == me) || {
                    let open = PlateRouting::build(topo, &self.params, &|owner, cell| {
                        if owner == me {
                            own.escape_level_km(cell)
                        } else {
                            Some(f64::MIN)
                        }
                    })?;
                    closed.same_on(&found.2, &open)
                        && catchment(&open)?.is_some_and(|other| {
                            other.2.iter().filter(|m| **m).count()
                                == found.2.iter().filter(|m| **m).count()
                        })
                };
                let (source, (targets, receivers, mask)) = if settled {
                    (Arc::new(closed), found)
                } else {
                    let full = self.drainage_routing(neighbor)?;
                    let Some(found) = catchment(&full)? else {
                        return Ok(Vec::new());
                    };
                    (full, found)
                };
                let upstream = self.handoff_inflow(&source, hops - 1, Some(&mask))?;
                let points: Vec<Vec2> = source
                    .nodes()
                    .iter()
                    .zip(&mask)
                    .filter(|(node, m)| **m && crate::runoff::needs_climate(node))
                    .map(|(node, _)| node.point)
                    .collect();
                let grid = crate::runoff::ClimateGrid::build(
                    &points,
                    self.params.drainage.climate_step_l,
                    |point| self.runoff_climate_at(point),
                )?;
                let runoff = crate::runoff::Runoff::calculate_with(
                    source.nodes(),
                    source.depressions(),
                    &grid,
                    &self.params,
                    Some(&upstream),
                    Some(&mask),
                )?;
                Ok(targets
                    .iter()
                    .zip(&receivers)
                    .map(|(&target, &here)| (here, runoff.nodes()[target].discharge_mm_l2))
                    .collect())
            };
        // Верхний уровень считает соседей одновременно; вложенные вызовы —
        // по очереди, чтобы число потоков не росло с глубиной. Вклады
        // складываются в порядке id соседей, от потоков сумма не зависит.
        let parts: Vec<Result<Vec<(usize, f64)>, RiverNetError>> =
            if wanted.is_none() && sources.len() >= 2 && worker_threads() >= 2 {
                std::thread::scope(|scope| {
                    let handles: Vec<_> = sources
                        .iter()
                        .map(|(neighbor, topo)| scope.spawn(|| contribution(neighbor, topo)))
                        .collect();
                    handles
                        .into_iter()
                        .map(|handle| handle.join().expect("поток притока не паникует"))
                        .collect()
                })
            } else {
                sources
                    .iter()
                    .map(|(neighbor, topo)| contribution(neighbor, topo))
                    .collect()
            };
        for part in parts {
            for (here, q) in part? {
                inflow[here] += q;
            }
        }
        Ok(inflow)
    }

    fn build_drainage_network(&self, plate: &Plate) -> Result<RiverNet, RiverNetError> {
        // Этапы целиком, по часам: вместе с работой над соседними плитами.
        let timings = crate::drainage::timings_enabled();
        let mut t = std::time::Instant::now();
        // Д-094: климат своих узлов суши зависит только от их координат,
        // а они известны уже по своей топографии. Считаем его заранее, пока
        // строятся топографии соседей и приток; сборка ниже возьмёт готовые
        // значения из кэша. На результат это не влияет: значения те же.
        let own = self.drainage_topo(plate)?;
        crate::drainage::log_phase(timings, "этап-своя", t);
        t = std::time::Instant::now();
        let warm_points: Vec<Vec2> = own
            .nodes()
            .iter()
            .filter(|node| {
                node.in_plate && node.route_km >= 0.0 && node.outlet != DrainageOutlet::Sea
            })
            .map(|node| node.point)
            .collect();
        let linked = std::thread::scope(|scope| {
            if worker_threads() >= 2 && !warm_points.is_empty() {
                scope.spawn(|| {
                    // Ошибку, если она есть, вернёт сборка сети.
                    let _ = crate::runoff::ClimateGrid::build(
                        &warm_points,
                        self.params.drainage.climate_step_l,
                        |point| self.runoff_climate_at(point),
                    );
                });
            }
            let routing = self.drainage_routing(plate)?;
            crate::drainage::log_phase(timings, "этап-связь", t);
            let t = std::time::Instant::now();
            let inflow =
                self.handoff_inflow(&routing, self.params.drainage.handoff_max_hops, None)?;
            crate::drainage::log_phase(timings, "этап-приток", t);
            Ok::<_, RiverNetError>((routing, inflow))
        });
        let (routing, inflow) = linked?;
        t = std::time::Instant::now();
        let net = RiverNet::assemble(
            routing,
            self.seed,
            &self.params,
            &|point| self.runoff_climate_at(point),
            &inflow,
        )?;
        crate::drainage::log_phase(timings, "этап-сборка", t);
        Ok(net)
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
        self.hydrology_queries(self.macro_point(x, z), false)
    }

    /// `with_levels` — R2 (§4): дописать в попадания уровни воды из сухой
    /// поверхности. Счёт `S` идёт с `false`, иначе уровни и `S` ссылались бы
    /// друг на друга.
    fn hydrology_queries(
        &self,
        point: Vec2,
        with_levels: bool,
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
        // Д-061: сеть, у которой рядом нет ни одного узла, в ответ не входит;
        // строить её ради этой точки не нужно.
        let owners = self.net_owners(point);
        for plate in plates_near(point, radius, self.seed, &self.params) {
            if owners.binary_search(&plate.id).is_err() {
                continue;
            }
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
            let mut channels_here = net.channels().query(point);
            // Д-127: путь полей (`with_levels = false`) всегда видит R1-озёра,
            // даже в `v1-dev-r2`. Слой воды (`true`) сохраняет свой предел.
            // Иначе высокое озеро меняет C/E/W против `v1-dev`.
            let mut lakes_here = if self.params.version.is_water_relief_r2() && !with_levels {
                net.lakes()
                    .query_with_limit(point, self.params.hydrology.r1_max_level_km)
            } else {
                net.lakes().query(point)
            };
            if with_levels && self.params.version.is_water_relief_r2() {
                // ВО1-1 (6.3): у `v1-dev-w` уровень реки — профиль от `S0`, у
                // R2 — прежний «низший берег минус 1». Озёра в ВО1–ВО2 у обеих
                // версий по правилу R2 (6.4).
                let water_update = self.params.version.is_water_update();
                let levels = if water_update {
                    self.w_plate_levels(&plate, &net)?
                } else {
                    self.r2_plate_levels(&plate, &net)?
                };
                if let Some(hit) = lakes_here.water_hit.as_mut() {
                    if water_update {
                        // ВО3-2 (§6.4): уровень озера от S0 — порог котловины на
                        // решётке стока, а не правило R2 (min(S) по контуру −1).
                        hit.level_blocks = hit
                            .level_s0_blocks
                            .map(|l| l + crate::height::SEA_LEVEL as f64);
                    } else {
                        hit.level_blocks = levels.lakes.get(hit.lake).copied().flatten();
                    }
                }
                if let Some(hit) = channels_here.nearest.as_mut() {
                    let reach = hit.reach;
                    let first = net.channels().reach_first_segment(reach);
                    let index = hit.segment.saturating_sub(first);
                    let along = hit.along;
                    let mirror = |up: f64, down: f64| up + (down - up) * along;
                    if water_update {
                        let profile = self.w_reach_profile(&levels, &net, reach)?;
                        hit.level_blocks = profile
                            .levels
                            .get(index)
                            .copied()
                            .flatten()
                            .map(|(up, down)| mirror(up, down));
                        // п. 7 (6.3): классы сегмента — классы звена.
                        if let Some(class) = profile.classes.get(index) {
                            hit.valley_class = class.valley;
                            hit.flow_class = class.flow;
                            hit.gp = class.gp;
                        }
                    } else {
                        hit.level_blocks = self
                            .r2_reach_levels(&levels, &net, reach)?
                            .get(index)
                            .copied()
                            .flatten()
                            .map(|(up, down)| mirror(up, down));
                    }
                }
                if water_update {
                    // 6.5: до трёх звеньев в радиусе с уровнями и классами —
                    // основа вырезки нескольких долин. Копия ближайшего допустима.
                    let mut extras = Vec::with_capacity(3);
                    for mut hit in net.channels().query_all(point, 3) {
                        let first = net.channels().reach_first_segment(hit.reach);
                        let index = hit.segment.saturating_sub(first);
                        let along = hit.along;
                        let profile = self.w_reach_profile(&levels, &net, hit.reach)?;
                        hit.level_blocks = profile
                            .levels
                            .get(index)
                            .copied()
                            .flatten()
                            .map(|(up, down)| up + (down - up) * along);
                        if let Some(class) = profile.classes.get(index) {
                            hit.valley_class = class.valley;
                            hit.flow_class = class.flow;
                            hit.gp = class.gp;
                        }
                        extras.push(hit);
                    }
                    channels_here.extra.absorb(extras);
                }
            }
            channels.merge(channels_here);
            lakes.merge(lakes_here);
        }
        Ok((channels, lakes))
    }

    /// R2 (PLAN-WATER-FABRIC §4): уровни воды плиты. Озёра — сразу, русла — по
    /// звеньям стока, лениво: у плиты тысячи сегментов, и первая же колонка
    /// платила бы за все (замер В5: 180 мс на чанк против бюджета Д-090).
    fn r2_plate_levels(
        &self,
        plate: &Plate,
        net: &crate::drainage::RiverNet,
    ) -> Result<Arc<PlateLevels>, RiverNetError> {
        self.r2_levels
            .get_or_build(plate.id, || self.build_r2_plate_levels(net))
    }

    /// Уровни одного звена стока: считается один раз на звено.
    fn r2_reach_levels(
        &self,
        levels: &PlateLevels,
        net: &crate::drainage::RiverNet,
        reach: usize,
    ) -> Result<ReachLevels, RiverNetError> {
        // Блокировку держим только на поиск и вставку: колонки чанка идут
        // параллельно, а ответ — чистая функция, повторный счёт не вреден.
        if let Some(found) = levels
            .reaches
            .lock()
            .expect("уровни русел R2 не отравлены")
            .get(&reach)
        {
            return Ok(Arc::clone(found));
        }
        let built = Arc::new(self.build_reach_levels(net, reach)?);
        levels
            .reaches
            .lock()
            .expect("уровни русел R2 не отравлены")
            .insert(reach, Arc::clone(&built));
        Ok(built)
    }

    /// §4, п. 1: зеркало озера — `min(S)` по контуру минус 1, плоское на всей
    /// площади. Считается в блоках, а не в километрах: таблица высот игры с
    /// километрами не связана.
    fn build_r2_plate_levels(
        &self,
        net: &crate::drainage::RiverNet,
    ) -> Result<PlateLevels, RiverNetError> {
        let l = self.params.block.l_blocks;
        let shift = self.spawn_shift;
        let to_blocks = |p: Vec2| ((p.x - shift.x) * l, (p.z - shift.z) * l);
        let mut levels = PlateLevels::default();
        let lakes = net.lakes().lakes();
        levels.lakes.reserve(lakes.len());
        for lake in lakes {
            let mut level = None;
            if lake.kind.has_water() && lake.level_km.is_some() {
                if let Some(ring) = lake.rings.first() {
                    let mut lowest: Option<f64> = None;
                    let mut last: Option<(f64, f64)> = None;
                    let mut taken = 0_usize;
                    for &point in ring {
                        let here = to_blocks(point);
                        let due = last.is_none_or(|(px, pz)| {
                            let dx = here.0 - px;
                            let dz = here.1 - pz;
                            (dx * dx + dz * dz).sqrt() >= LAKE_RING_STEP_BLOCKS
                        });
                        if !due {
                            continue;
                        }
                        last = Some(here);
                        taken += 1;
                        if let Some(s) = self.dry_surface_at(here.0, here.1)? {
                            lowest = Some(lowest.map_or(s, |old: f64| old.min(s)));
                        }
                        if taken >= LAKE_RING_MAX_SAMPLES {
                            break;
                        }
                    }
                    // Зеркало ниже самой низкой точки берега на блок: вода
                    // стоит на `level − 1`, и берег не оказывается под водой.
                    level = lowest
                        .map(|s| (s - 1.0).floor())
                        .filter(|value| *value > crate::height::SEA_LEVEL);
                }
            }
            levels.lakes.push(level);
        }
        Ok(levels)
    }

    /// §4, п. 2: уровень русла — низший берег сечения минус 1, только вниз по
    /// течению. Считается на звено стока целиком: бюджет Д-090 — на звено.
    fn build_reach_levels(
        &self,
        net: &crate::drainage::RiverNet,
        reach: usize,
    ) -> Result<Vec<Option<(f64, f64)>>, RiverNetError> {
        let l = self.params.block.l_blocks;
        let shift = self.spawn_shift;
        let to_blocks = |p: Vec2| ((p.x - shift.x) * l, (p.z - shift.z) * l);
        let channels = net.channels();
        let mut segments = vec![None; channels.reach_segments(reach).len()];
        // Уровень вершины — низший берег минус 1; вниз по течению только
        // убывает, но не быстрее блока на блок русла.
        let mut current: Option<f64> = None;
        let mut target: Option<f64> = None;
        let mut last_sample: Option<Vec2> = None;
        let started = std::time::Instant::now();
        let stride_l = RIVER_LEVEL_STRIDE_BLOCKS / l;
        for (offset, segment) in channels.reach_segments(reach).iter().enumerate() {
            let a = segment.a;
            let b = segment.b;
            let direction = b.point.sub(a.point);
            // Запасной режим (Д-090): вершины реже, через 8 блоков, уровень
            // между ними держится прежним и остаётся монотонным.
            let slow = started.elapsed().as_millis() > RIVER_LEVEL_BUDGET_MS;
            let due = last_sample.is_none_or(|last| {
                let step = if slow { stride_l } else { 0.0 };
                b.point.sub(last).len() >= step
            });
            if due && direction.len() > 1e-12 {
                last_sample = Some(b.point);
                // Низший из двух берегов сечения: `core/2 + 1` блок от оси.
                let normal = direction.normalized().perp();
                let offset_l = (0.5 * b.core_l * l + 1.0) / l;
                let mut bank: Option<f64> = None;
                for side in [1.0, -1.0] {
                    let probe = b.point.add(normal.scale(side * offset_l));
                    let (px, pz) = to_blocks(probe);
                    if let Some(s) = self.dry_surface_at(px, pz)? {
                        bank = Some(bank.map_or(s, |old: f64| old.min(s)));
                    }
                }
                if let Some(bank) = bank {
                    target = Some((bank - 1.0).floor());
                }
            }
            let start = current.or(target).unwrap_or(crate::height::SEA_LEVEL);
            let desired = target.map_or(start, |level| start.min(level));
            // Падение внутри сегмента размазано по его длине (уровень
            // интерполируется по `along`), поэтому ступень между соседними
            // блоками — не больше блока на блок. Быстрее рельеф падает только
            // на водопадах; их считают метрики (§4, п. 2).
            let end = desired.min(start);
            current = Some(end);
            if let Some(slot) = segments.get_mut(offset) {
                *slot = if start > crate::height::SEA_LEVEL {
                    Some((start, end))
                } else {
                    None
                };
            }
        }
        Ok(segments)
    }

    /// ВО1-1 (6.3): уровни `W` плиты. Озёра — правило R2 (6.4, ВО1–ВО2),
    /// русла — профиль от `S0`, лениво по звену: у плиты тысячи сегментов.
    fn w_plate_levels(
        &self,
        plate: &Plate,
        net: &crate::drainage::RiverNet,
    ) -> Result<Arc<PlateLevels>, RiverNetError> {
        self.w_levels
            .get_or_build(plate.id, || self.build_w_plate_levels(net))
    }

    /// п. 3/8 (6.3, «порядок звеньев — сверху вниз по течению»): итеративный
    /// постфиксный обход предков звена. Вход звена собирается из концов
    /// родителей, а в длинной реке родителей — целая цепочка: рекурсивный
    /// обход переполнял бы стек на больших сетях (звенья стоят друг в друге
    /// полными сборками профилей). Здесь обход явный: предки собираются в
    /// расширенном порядке обхода и строятся в обратном, от истоков вниз по
    /// течению; после этого сборка каждого звена — без рекурсии вглубь.
    /// Порядок построения звеньев больше не влияет на значения (уровни
    /// концов `w_ends` — пред-подпорные и не переписываются).
    fn w_build_ancestors(
        &self,
        levels: &PlateLevels,
        net: &crate::drainage::RiverNet,
        reach: usize,
    ) {
        // Постфиксный (топологический) обход вверх по течению: сначала
        // истоки, потом их дети. Обратный расширенный порядок не является
        // топологическим при слияниях рек (ромбовидная ветвь) и может
        // потерять звено, не дождавшись его предка — поэтому обход явный,
        // со спуск/возврат-маркерами.
        let reaches = net.channels().reaches();
        let mut seen = vec![false; reaches.len()];
        let mut post = Vec::new();
        let mut stack = vec![(reach, false)];
        while let Some((r, done)) = stack.pop() {
            if done {
                post.push(r);
                continue;
            }
            if seen[r] {
                continue;
            }
            seen[r] = true;
            stack.push((r, true));
            for &parent in &levels.w_parents[r] {
                if !seen[parent] {
                    stack.push((parent, false));
                }
            }
        }
        // Предки чаще уже построены; строим в топологическом порядке — от
        // истоков вниз по течению. Звено строим напрямую (не через
        // `w_reach_profile`): предки уже построены на этом же проходе,
        // сборка не требует рекурсии.
        for &r in &post {
            let built = levels
                .w_profiles
                .lock()
                .expect("профиль русел W не отравлен")
                .contains_key(&r);
            if built {
                continue;
            }
            // Звено, которое уже стоит на стеке сборки (цикл или собирается
            // соседним потоком), здесь не строим: его соберёт контур по
            // запросу либо соседний поток. Без отметки цикл вернулся бы в
            // помеченное звено второй раз.
            let mine = std::thread::current().id();
            let marked = levels
                .w_in_progress
                .lock()
                .expect("путь сборки W не отравлен")
                .insert(r, mine)
                .is_none();
            if !marked {
                // Гонка: между проверкой и вставкой отметку взял чужой поток.
                continue;
            }
            // Ошибка звена не рвет обход: пустое звено — нормальное значение,
            // а наверх ошибку поднимет сборка по запросу.
            let build_result = self.build_w_reach_levels(levels, net, r);
            levels
                .w_in_progress
                .lock()
                .expect("путь сборки W не отравлен")
                .remove(&r);
            if let Ok(built) = build_result {
                levels
                    .w_profiles
                    .lock()
                    .expect("профиль русел W не отравлен")
                    .insert(r, Arc::new(built));
            }
        }
    }

    /// Профиль одного звена: считается один раз, уровни и классы заодно.
    /// Звено, которое уже стоит на стеке сборки (цикл в сети стока), получает
    /// `RiverNetError::Cycle` вместо бесконечной взаимной рекурсии — такую
    /// ветвь `w_parent_end` считает головой реки (уровень берётся из
    /// оставшихся притоков или из приёмника).
    fn w_reach_profile(
        &self,
        levels: &PlateLevels,
        net: &crate::drainage::RiverNet,
        reach: usize,
    ) -> Result<ReachProfiles, RiverNetError> {
        // Отметка с владельцем. `Cycle` — только при повторном входе того же
        // потока (настоящий цикл в сети стока). Чужой поток, который уже
        // собирает это звено, собирает независимо (как до отметчика):
        // значение то же (сборка чистая), чужая отметка не трогается.
        let mine = std::thread::current().id();
        let holder = {
            let marks = levels
                .w_in_progress
                .lock()
                .expect("путь сборки W не отравлен");
            marks.get(&reach).copied()
        };
        if let Some(owner) = holder {
            if owner == mine {
                // Цикл: звено уже стоит на нашем стеке.
                let node = net
                    .channels()
                    .reaches()
                    .get(reach)
                    .map(|r| r.source_node)
                    .unwrap_or(reach);
                return Err(RiverNetError::Cycle { node });
            }
            // Чужой поток: собираем без своей метки, чужую не трогаем.
            return self.w_reach_profile_inner(levels, net, reach);
        }
        // Гонка с соседним потоком: между проверкой и вставкой отметку мог
        // занять другой поток — тогда собираем параллельно, не трогая её.
        let won = {
            let mut marks = levels
                .w_in_progress
                .lock()
                .expect("путь сборки W не отравлен");
            match marks.entry(reach) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(mine);
                    true
                }
                std::collections::hash_map::Entry::Occupied(_) => false,
            }
        };
        if !won {
            return self.w_reach_profile_inner(levels, net, reach);
        }
        let out = self.w_reach_profile_inner(levels, net, reach);
        levels
            .w_in_progress
            .lock()
            .expect("путь сборки W не отравлен")
            .remove(&reach);
        out
    }

    fn w_reach_profile_inner(
        &self,
        levels: &PlateLevels,
        net: &crate::drainage::RiverNet,
        reach: usize,
    ) -> Result<ReachProfiles, RiverNetError> {
        if let Some(found) = levels
            .w_profiles
            .lock()
            .expect("профиль русел W не отравлен")
            .get(&reach)
        {
            return Ok(Arc::clone(found));
        }
        // Сначала предки (без рекурсии) — иначе длинная цепочка звеньев
        // переполнила бы стек полными сборками друг в друге.
        self.w_build_ancestors(levels, net, reach);
        if let Some(found) = levels
            .w_profiles
            .lock()
            .expect("профиль русел W не отравлен")
            .get(&reach)
        {
            return Ok(Arc::clone(found));
        }
        let built = Arc::new(self.build_w_reach_levels(levels, net, reach)?);
        levels
            .w_profiles
            .lock()
            .expect("профиль русел W не отравлен")
            .insert(reach, Arc::clone(&built));
        Ok(built)
    }

    /// Озёра `W` пока по правилу R2 (6.4): их поверхность — ещё не `S0`.
    /// Русла считаются лениво в профиле звена (`w_reach_profile`); карта
    /// родителей нужна обходу предков без пересканирования списка.
    fn build_w_plate_levels(
        &self,
        net: &crate::drainage::RiverNet,
    ) -> Result<PlateLevels, RiverNetError> {
        let mut levels = self.build_r2_plate_levels(net)?;
        levels.w_parents = w_parents_of(net);
        Ok(levels)
    }

    /// ВО1-1 (6.3, пп. 1–6, 7, 8): профиль уровня реки `W` по звену.
    ///
    /// Пробы оси идут с шагом `water.profile_step_blocks` (при перерасходе
    /// бюджета шаг удваивается): цель `T = floor(B) − 1` от низшего из трёх
    /// `S0` сечения, не ниже моря. Вниз по течению уровень только убывает
    /// (бегущий минимум), поэтому ступень внутри звена — не больше блока на
    /// блок русла; водопады (п. 6) только размечаются (`FallTop`), перепад
    /// раскладывается на ступени как в R2. Приёмник (п. 4): подпор вверх по
    /// течению, висячий приток вне холодных гор выравнивается уклоном
    /// `grade_slope`. Вход звена (п. 3) — низший из родителей, включая
    /// передачи от соседних плит (п. 8). Классы долины (5.2) и потока (5.3)
    /// пишутся в профиль (п. 7).
    fn build_w_reach_levels(
        &self,
        levels: &PlateLevels,
        net: &crate::drainage::RiverNet,
        reach: usize,
    ) -> Result<WReachProfile, RiverNetError> {
        let l = self.params.block.l_blocks;
        let shift = self.spawn_shift;
        let to_blocks = |p: Vec2| ((p.x - shift.x) * l, (p.z - shift.z) * l);
        let channels = net.channels();
        let water = &self.params.water;
        let sea = crate::height::SEA_LEVEL;
        let source = channels.reach_segments(reach);
        if source.is_empty() {
            return Ok(WReachProfile {
                levels: Vec::new(),
                classes: Vec::new(),
            });
        }
        let n = source.len();
        // Длина оси по вершинам в блоках: окно уклона и сглаживание класса.
        let mut cum_blocks: Vec<f64> = Vec::with_capacity(n);
        let mut acc = 0.0_f64;
        for segment in source {
            acc += segment.b.point.sub(segment.a.point).len() * l;
            cum_blocks.push(acc);
        }
        // п. 3/8 (6.3): уровень входа — от притока (п. 4) или от верхней плиты.
        let entry = self.w_entry_level(levels, net, reach)?;
        // п. 1–2: берег `B`, проба долины `Gp` и входы классов в пробах оси.
        // Между пробами уровень держится прежним, поэтому ступень не больше
        // блока на блок.
        let mut banks: Vec<Option<f64>> = vec![None; n];
        let mut targets: Vec<Option<f64>> = vec![None; n];
        let mut gp: Vec<Option<f64>> = vec![None; n];
        let mut inputs: Vec<Option<crate::water_profile::ValleyClassInput>> = vec![None; n];
        let mut last_sample: Option<Vec2> = None;
        let started = std::time::Instant::now();
        for (index, segment) in source.iter().enumerate() {
            let b = segment.b;
            let direction = b.point.sub(segment.a.point);
            if direction.len() <= 1e-12 {
                continue;
            }
            // п. 1: при перерасходе бюджета плиты шаг проб удваивается.
            let doubled = started.elapsed().as_millis() > u128::from(water.profile_budget_ms);
            let step_l = water.profile_step_blocks * if doubled { 2.0 } else { 1.0 } / l;
            let due = last_sample.is_none_or(|last| b.point.sub(last).len() >= step_l);
            if !due {
                continue;
            }
            last_sample = Some(b.point);
            // п. 1: ось, нормаль и полуширина воды; берег — низший из трёх `S0`.
            let normal = direction.normalized().perp();
            let edge_l = 0.5 * b.core_l + water.bank_probe_blocks / l;
            let mut bank: Option<f64> = None;
            let mut center: Option<WProbe> = None;
            for offset_l in [0.0, edge_l, -edge_l] {
                let (px, pz) = to_blocks(b.point.add(normal.scale(offset_l)));
                if let Some(probe) = self.w_probe(px, pz) {
                    bank = Some(bank.map_or(probe.macro_y, |old: f64| old.min(probe.macro_y)));
                    if offset_l == 0.0 {
                        center = Some(probe);
                    }
                }
            }
            // п. 1: две пробы для ширины долины `Gp`, `R_v = h + 2 + R_wall`.
            let rv_l = 0.5 * b.core_l + (2.0 + water.wall_reach_blocks) / l;
            let mut gp_max: Option<f64> = None;
            for offset_l in [rv_l, -rv_l] {
                let (px, pz) = to_blocks(b.point.add(normal.scale(offset_l)));
                if let Some(probe) = self.w_probe(px, pz) {
                    gp_max = Some(gp_max.map_or(probe.macro_y, |old: f64| old.max(probe.macro_y)));
                }
            }
            gp[index] = gp_max;
            if let Some(bank) = bank {
                banks[index] = Some(bank);
                // п. 2: цель — блок под низшим берегом, не ниже моря.
                let target = bank.floor() - 1.0;
                targets[index] = Some(target.max(sea));
            }
            if let Some(center) = center {
                // Входы класса долины: глубина и уклон заполняются после
                // профиля (ниже), макро-величины известны уже здесь.
                inputs[index] = Some(crate::water_profile::ValleyClassInput {
                    cold_mountain_weight: center.cold_mountain_weight,
                    polar_belt: center.t_c < 3.0,
                    erosion_e0_e1: center.erosion_e < self.params.height.e_knots[2],
                    depth_blocks: 0.0,
                    aridity: center.aridity,
                    plateau: center.plateau,
                    slope: 0.0,
                });
            }
        }
        // п. 3 и 5: бегущий минимум вниз по течению от уровня входа.
        let mut level: Vec<f64> = vec![sea; source.len()];
        let mut current: Option<f64> = entry;
        for index in 0..source.len() {
            let start = current.or(targets[index]).unwrap_or(sea);
            let here = match targets[index] {
                Some(target) => start.min(target),
                None => start,
            };
            current = Some(here);
            level[index] = here;
        }
        // п. 3/8: конец звена ДО коррекции приёмника — в общую ячейку.
        // Приёмник (п. 4) и соседние слияния читают его отсюда вместо
        // повторной сборки звена, которое всё ещё стоит на стеке.
        let end_level = *level.last().expect("звено непусто");
        levels
            .w_ends
            .lock()
            .expect("уровни концов W не отравлены")
            .entry(reach)
            .or_insert(end_level);
        if let Some(receiver) = self.w_receiver_level(levels, net, reach, Some(end_level))? {
            let mouth_level = *level.last().expect("звено непусто");
            if mouth_level + 1.0 < receiver {
                // Подпор: вода у устья ниже приёмника — поднимаем вверх по
                // течению, пока берег не окажется ниже зеркала приёмника.
                let mut index = level.len();
                while index > 0 {
                    index -= 1;
                    level[index] = level[index].max(receiver);
                    if banks[index].is_some_and(|bank| bank < receiver + 1.0) {
                        break;
                    }
                }
            } else if mouth_level > receiver + 1.0 && !self.w_cold_mountains(net, reach) {
                // п. 4: приток висит выше приёмника — выравниваем низовье
                // уклоном `grade_slope` от устья вверх. В холодных горах
                // ступень остаётся: это висячая долина (водопады, п. 6).
                let mut distance = 0.0_f64;
                for index in (0..level.len()).rev() {
                    let segment = &source[index];
                    distance += segment.b.point.sub(segment.a.point).len() * l;
                    let cap = receiver + (distance * water.grade_slope).floor();
                    level[index] = level[index].min(cap);
                }
            }
        }
        // п. 7 / 5.3: уклон окна 32 блока русла (±16 блоков от пробы).
        let half_window = 16.0_f64;
        let mut slope: Vec<f64> = vec![0.0; n];
        for i in 0..n {
            let center = cum_blocks[i];
            let lo = cum_blocks.partition_point(|&x| x < center - half_window);
            let hi = cum_blocks
                .partition_point(|&x| x <= center + half_window)
                .saturating_sub(1);
            let a = lo.min(i);
            let b = hi.max(i);
            if b > a {
                let span = cum_blocks[b] - cum_blocks[a];
                if span > 1e-6 {
                    slope[i] = ((level[a] - level[b]) / span).max(0.0);
                }
            }
        }
        // п. 7 / 5.2: класс долины; пробы без `Gp` наследуют ближайший класс.
        let mut raw: Vec<crate::water_profile::ValleyClass> =
            vec![crate::water_profile::ValleyClass::Plain; n];
        for i in 0..n {
            if let Some(input) = inputs[i] {
                let mut valley_input = input;
                valley_input.depth_blocks = gp[i].map(|g| (g - level[i]).max(0.0)).unwrap_or(0.0);
                valley_input.slope = slope[i];
                raw[i] = crate::water_profile::valley_class(valley_input, water);
            }
        }
        let mut carry = None;
        for i in 0..n {
            if inputs[i].is_some() {
                carry = Some(raw[i]);
            } else if let Some(value) = carry {
                raw[i] = value;
            }
        }
        let mut carry = None;
        for i in (0..n).rev() {
            if inputs[i].is_some() {
                carry = Some(raw[i]);
            } else if let Some(value) = carry {
                raw[i] = value;
            }
        }
        let valley = crate::water_profile::smooth_valley_classes(
            &cum_blocks,
            &raw,
            water.class_window_blocks,
        );
        // п. 7 / 6.3: класс потока сверху вниз; каскад глушит водопад.
        let mut classes = Vec::with_capacity(n);
        let mut upstream = crate::water_profile::FlowClass::Calm;
        for i in 0..n {
            let drop_blocks = if i == 0 {
                0.0
            } else {
                (level[i - 1] - level[i]).max(0.0)
            };
            let flow = crate::water_profile::flow_class(slope[i], drop_blocks, upstream, water);
            classes.push(crate::water_profile::SegmentClass {
                valley: valley[i],
                flow,
                gp: gp[i].unwrap_or(0.0),
            });
            upstream = flow;
        }
        // Зеркало сегмента: `(уровень в начале, уровень в конце)`; между ними
        // интерполяция по `along`, поэтому ступень ≤ 1 блок на блок русла.
        let mut segments = vec![None; n];
        for index in 0..level.len() {
            let up = level[index];
            let down = level.get(index + 1).copied().unwrap_or(level[index]);
            if up > sea || down > sea {
                segments[index] = Some((up, down));
            }
        }
        Ok(WReachProfile {
            levels: segments,
            classes,
        })
    }

    /// `T` звена по его первой вершине: низший из трёх `S0` (ось и два
    /// берега), блок под ней, не ниже моря. Та же формула, что в начале
    /// профиля, но одна проба.
    fn w_first_target(&self, net: &crate::drainage::RiverNet, reach: usize) -> Option<f64> {
        let l = self.params.block.l_blocks;
        let shift = self.spawn_shift;
        let sea = crate::height::SEA_LEVEL;
        let water = &self.params.water;
        let channels = net.channels();
        let segment = channels.reach_segments(reach).first().copied()?;
        let b = segment.b;
        let direction = b.point.sub(segment.a.point);
        if direction.len() <= 1e-12 {
            return None;
        }
        let normal = direction.normalized().perp();
        let edge_l = 0.5 * b.core_l + water.bank_probe_blocks / l;
        let mut bank: Option<f64> = None;
        for offset_l in [0.0, edge_l, -edge_l] {
            let here = b.point.add(normal.scale(offset_l));
            let px = (here.x - shift.x) * l;
            let pz = (here.z - shift.z) * l;
            if let Some(probe) = self.w_probe(px, pz) {
                bank = Some(bank.map_or(probe.macro_y, |old: f64| old.min(probe.macro_y)));
            }
        }
        let bank = bank?;
        Some((bank.floor() - 1.0).max(sea))
    }

    /// п. 3/8 (6.3): уровень конца звена-родителя. Порядок источников:
    /// звено, которое строится прямо сейчас (`in_progress`) — его конец уже
    /// посчитан; затем ячейка `w_ends` — пред-подпорное значение, одинаковое
    /// для всех путей обхода; и только потом сборка звена (после неё ячейка
    /// всегда заполнена, либо звено пустое). Первые два источника не дают
    /// слияниям собирать профили друг друга заново, пока они стоят на стеке
    /// вызова (иначе — переполнение стека на слиянии двух притоков).
    /// Все три источника отдают пред-подпорный уровень: вход звена (п. 3)
    /// не зависит от порядка обхода и остаётся детерминированным.
    fn w_parent_end(
        &self,
        levels: &PlateLevels,
        net: &crate::drainage::RiverNet,
        reach: usize,
        in_progress: Option<(usize, f64)>,
    ) -> Result<Option<f64>, RiverNetError> {
        if let Some((active, end)) = in_progress {
            if active == reach {
                return Ok(Some(end));
            }
        }
        if let Some(end) = levels
            .w_ends
            .lock()
            .expect("уровни концов W не отравлены")
            .get(&reach)
        {
            return Ok(Some(*end));
        }
        match self.w_reach_profile(levels, net, reach) {
            Ok(_) => {}
            // Цикл вверх по течению: ветвь не задаёт уровень входа — считаем
            // её головой реки (уровень даёт вторая ветвь или приёмник).
            Err(RiverNetError::Cycle { .. }) => return Ok(None),
            Err(other) => return Err(other),
        }
        // После сборки ячейка заполнена; звено с пустым источником — None.
        if let Some(end) = levels
            .w_ends
            .lock()
            .expect("уровни концов W не отравлены")
            .get(&reach)
        {
            return Ok(Some(*end));
        }
        Ok(None)
    }

    /// п. 3/8 (6.3): уровень входа в звено — низший из родителей
    /// (`mouth_node == source_node`), включая передачи от соседних плит.
    /// `None` — голова реки. Подпор (п. 4) сюда не входит: он смотрит вниз
    /// по течению и на верхнюю плиту не переходит.
    fn w_entry_level(
        &self,
        levels: &PlateLevels,
        net: &crate::drainage::RiverNet,
        reach: usize,
    ) -> Result<Option<f64>, RiverNetError> {
        if let Some(found) = levels
            .w_entries
            .lock()
            .expect("уровни входа W не отравлены")
            .get(&reach)
        {
            return Ok(*found);
        }
        let entry = self.w_entry_level_inner(levels, net, reach, None)?;
        levels
            .w_entries
            .lock()
            .expect("уровни входа W не отравлены")
            .insert(reach, entry);
        Ok(entry)
    }

    /// `in_progress`: (номер звена в этой сети, его уровень на конце) —
    /// звено, которое строится прямо сейчас и чей конец уже известен.
    /// Без этой пометки подпор (п. 4) и вход звена (п. 8) собирали бы друг друга
    /// заново: `A → receiver(B) → entry(B) → parents incl. A → build(A)` — цикл.
    fn w_entry_level_inner(
        &self,
        levels: &PlateLevels,
        net: &crate::drainage::RiverNet,
        reach: usize,
        in_progress: Option<(usize, f64)>,
    ) -> Result<Option<f64>, RiverNetError> {
        let channels = net.channels();
        let Some(this) = channels.reaches().get(reach) else {
            return Ok(None);
        };
        let source = this.source_node;
        let Some(&node) = net.nodes().get(source) else {
            return Ok(None);
        };
        let node_point = node.point;
        let mut best: Option<f64> = None;
        // Внутриплитовые родители: звенья той же сети, чей рот — наш исток.
        for (index, candidate) in channels.reaches().iter().enumerate() {
            if index == reach || candidate.mouth_node != source {
                continue;
            }
            if let Some(level) = self.w_parent_end(levels, net, index, in_progress)? {
                best = Some(best.map_or(level, |old: f64| old.min(level)));
            }
        }
        // Межплитовые: сети соседних плит, у которых звено заканчивается
        // передачей (`Handoff`) в этот узел.
        for plate in plates_near(node_point, 2.0, self.seed, &self.params) {
            if plate.id == net.plate_id() {
                continue;
            }
            let Ok(other_net) = self.drainage_network(&plate) else {
                continue;
            };
            let other_levels = self.w_plate_levels(&plate, &other_net)?;
            for (index, candidate) in other_net.channels().reaches().iter().enumerate() {
                if candidate.end != crate::channel::ReachEnd::Handoff {
                    continue;
                }
                let Some(&mouth) = other_net.nodes().get(candidate.mouth_node) else {
                    continue;
                };
                if mouth.point.sub(node_point).len() > 1e-9 {
                    continue;
                }
                if let Some(level) = self.w_parent_end(&other_levels, &other_net, index, None)? {
                    best = Some(best.map_or(level, |old: f64| old.min(level)));
                }
            }
        }
        Ok(best)
    }

    /// п. 4 (6.3): уровень приёмника звена, блоки. Море — 63, озеро — его
    /// зеркало R2 (6.4), слияние — уровень звена, которое начинается от того
    /// же узла (оно ниже по течению, поэтому рекурсия конечна). У прочих
    /// концов приёмника нет.
    /// `this_end`: уровень строительного звена на конце (pre-подпор) — нужен
    /// ветви `Confluence`, чтобы не собирать его родителя заново.
    fn w_receiver_level(
        &self,
        levels: &PlateLevels,
        net: &crate::drainage::RiverNet,
        reach: usize,
        this_end: Option<f64>,
    ) -> Result<Option<f64>, RiverNetError> {
        let channels = net.channels();
        let Some(this) = channels.reaches().get(reach) else {
            return Ok(None);
        };
        match this.end {
            crate::channel::ReachEnd::Sea => Ok(Some(crate::height::SEA_LEVEL)),
            crate::channel::ReachEnd::Lake => {
                let Some(last) = channels.reach_segments(reach).last() else {
                    return Ok(None);
                };
                let index = net
                    .lakes()
                    .query(last.b.point)
                    .water_hit
                    .map(|hit| hit.lake);
                Ok(index.and_then(|index| levels.lakes.get(index).copied().flatten()))
            }
            crate::channel::ReachEnd::Confluence => {
                let mouth = this.mouth_node;
                let next = channels
                    .reaches()
                    .iter()
                    .position(|candidate| candidate.source_node == mouth);
                let Some(next) = next else {
                    return Ok(None);
                };
                if next == reach {
                    return Ok(None);
                }
                // Уровень начала звена ниже по течению — формула входа (п. 3):
                // минимум цели по первой вершине и концам его родителей.
                // Звено `reach` уже строится, его конец — `this_end`.
                let target = self.w_first_target(net, next);
                let in_progress = this_end.map(|end| (reach, end));
                let entry = self.w_entry_level_inner(levels, net, next, in_progress)?;
                let mut best: Option<f64> = None;
                for value in [target, entry].into_iter().flatten() {
                    best = Some(best.map_or(value, |old: f64| old.min(value)));
                }
                Ok(best)
            }
            _ => Ok(None),
        }
    }

    /// В холодных горах (вес морфозоны 6 ≥ 0,5) низовье притока не
    /// выравнивается: висячая долина — это будущий водопад (6.3, п. 6).
    fn w_cold_mountains(&self, net: &crate::drainage::RiverNet, reach: usize) -> bool {
        let channels = net.channels();
        let Some(last) = channels.reach_segments(reach).last() else {
            return false;
        };
        let l = self.params.block.l_blocks;
        let shift = self.spawn_shift;
        let x = (last.b.point.x - shift.x) * l;
        let z = (last.b.point.z - shift.z) * l;
        self.morpho_weights(x, z)
            .is_some_and(|weights| weights.values[6] >= 0.5)
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
        let geometry = self.cached_hydrology_queries(x, z, false)?;
        Ok(hydrology::resolve(
            geometry.channels,
            geometry.lakes,
            &climate,
            &self.params,
        ))
    }

    /// Гидрология с уровнями воды R2 (§4) — для диагностики и проверок.
    /// `hydrology_at` остаётся сырым: по нему считается `S`, и уровни в нём
    /// замкнули бы «уровень → `S` → уровень».
    pub fn hydrology_levels_at(&self, x: f64, z: f64) -> Result<HydrologySample, RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        hydrology::validate(&self.params)?;
        let climate = self
            .climate_sample(x, z)
            .ok_or(RiverNetError::InvalidInput { cell: (0, 0) })?;
        let geometry = self.cached_hydrology_queries(x, z, true)?;
        Ok(hydrology::resolve(
            geometry.channels,
            geometry.lakes,
            &climate,
            &self.params,
        ))
    }

    fn cached_hydrology_queries(
        &self,
        x: f64,
        z: f64,
        with_levels: bool,
    ) -> Result<HydrologySample, RiverNetError> {
        // Сначала проверяем версию/параметры, затем строим сети вне замков.
        if !self.params.version.is_water_relief_dev() {
            return Err(RiverNetError::UnsupportedVersion(self.params.version));
        }
        hydrology::validate(&self.params)?;
        let key = (x.to_bits() as i64, z.to_bits() as i64);
        let cache = if with_levels {
            &self.hydrology_levels
        } else {
            &self.hydrology
        };
        if let Some(hit) = cache.get(key) {
            return Ok(*hit);
        }
        let (channels, lakes) = self.hydrology_queries(self.macro_point(x, z), with_levels)?;
        // Климат зависит от пути (reference/fast), поэтому кэшируем только
        // геометрию. Хранение готового kind здесь смешало бы два пути.
        let geometry = HydrologySample {
            channels,
            lakes,
            networks_considered: channels.networks_considered,
            ..HydrologySample::default()
        };
        cache.get_or_insert(key, || Arc::new(geometry));
        Ok(geometry)
    }

    /// Класс до расчёта высоты. На суше `v1-dev` биом выбирается заново
    /// после высоты (К5, Д-111), поэтому здесь хватает заготовки без шума
    /// дрожания границ: она никуда не попадает.
    fn class_before_height(
        &self,
        climate: &ClimateSample,
        is_ocean: bool,
        x: f64,
        z: f64,
    ) -> crate::classify::ClassSample {
        if self.params.version.is_water_relief_dev() && !is_ocean {
            crate::classify::classify_inputs_at(
                climate.temperature.t_c,
                climate.temperature.t_bio,
                climate.precip.ratio,
                climate.temperature.t_min,
                &self.params,
            )
        } else {
            classify(climate, is_ocean, x, z, self.seed, &self.params)
        }
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
        apply_water: bool,
    ) -> Result<SurfaceFields, RiverNetError> {
        if !self.params.version.is_water_relief_dev() {
            let vanilla =
                crate::vanilla::sample(climate, &class, x, z, self.seed, &self.params, local_river);
            let pv = crate::vanilla::peaks_valleys(vanilla.w);
            return Ok(SurfaceFields {
                climate: *climate,
                class,
                vanilla,
                river: local_river,
                hydrology: HydrologySample::default(),
                is_ocean,
                pv,
                height: crate::height::HeightSample::default(),
            });
        }
        let geometry = self.cached_hydrology_queries(x, z, apply_water)?;
        let water = hydrology::resolve(geometry.channels, geometry.lakes, climate, &self.params);
        // §4, п. 5 (Д-127): `v1-dev-r2` меняет только слой воды. Поля и рельеф
        // обязаны совпасть с `v1-dev` побитово, поэтому C/E/W/биом и `S`
        // считаются по гидрологии с пределом R1, а версия с уровнями идёт
        // только в `with_water` и в `SurfaceFields.hydrology`. Иначе высокое
        // озеро в R2 перестаёт быть `DeferredWater`, меняет сушу против
        // `v1-dev` — и вода стоит на земле, которой мир не строил.
        // Геометрия для полей и для слоя воды разная: `geometry` (с уровнями)
        // остаётся версионной, а поля R2 берут R1-геометрию из кэша `false`.
        let terrain_water = if self.params.version.is_water_relief_r2() {
            let fields_geometry = self.cached_hydrology_queries(x, z, false)?;
            hydrology::resolve_with_limit(
                fields_geometry.channels,
                fields_geometry.lakes,
                climate,
                &self.params,
                self.params.hydrology.r1_max_level_km,
            )
        } else {
            water
        };
        // Поля всегда строятся по R1-гидрологии, поэтому здесь обычные
        // `apply_fields`/`classify_surface`: настоящие болота дают swamp_e,
        // а высокое озеро уже стало `DeferredWater` и водного биома не даёт.
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
        let shore_dist = self.shore_distance_blocks(
            x,
            z,
            continentalness(crust.e_km, crust.iota_l, &self.params),
        );
        let shore_fade = self.shore_fade_from(shore_dist);
        let relief_sample =
            relief::sample_near_shore(&plate, &crust, depth, shore_fade, self.seed, &self.params);
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
        let vanilla = hydrology::apply_fields(base, local_river, &terrain_water, &self.params);
        let class = hydrology::classify_surface(class, &terrain_water, climate, &self.params);
        let river = local_river.max(terrain_water.river);
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
        let shore_dist = if vanilla.c <= self.params.climate.ocean_c_threshold {
            0.0
        } else {
            shore_dist
        };
        let water_update = self.params.version.is_water_update();
        // R2 (§4): `S` — сухая поверхность колонки. Уровень воды считается от
        // неё (`dry_surface_at`), поэтому вырез только опускает рельеф.
        // W (6.1/6.5): `S0` считается БЕЗ гидрологии (pv_relief, e_km), вырез
        // ниже — профилем от `S0` (`with_water_w`).
        let dry = if water_update {
            crate::height::sample_micro(
                vanilla.c,
                vanilla.e,
                pv_relief,
                e_km,
                morpho.values[6],
                morpho.values[7],
                micro,
                1.0,
                shore_dist,
                &self.params,
            )
        } else {
            crate::height::sample_micro(
                vanilla.c,
                vanilla.e,
                pv,
                e_valley_km,
                morpho.values[6],
                morpho.values[7],
                micro,
                1.0 - crate::math::clamp01(river),
                shore_dist,
                &self.params,
            )
        };
        let height = if apply_water {
            if water_update {
                // Вклад микробугров — отдельное слагаемое (6.1): разность
                // двух вызовов; `gate` гасит бугры у воды в `with_water_w`.
                let macro_h = crate::height::sample_micro(
                    vanilla.c,
                    vanilla.e,
                    pv_relief,
                    e_km,
                    morpho.values[6],
                    morpho.values[7],
                    0.0,
                    0.0,
                    shore_dist,
                    &self.params,
                );
                let micro_dy = dry.surface_y - macro_h.surface_y;
                crate::height::with_water_w(macro_h, micro_dy, &water, &self.params)
            } else {
                crate::height::with_water(dry, &water, &self.params)
            }
        } else {
            dry
        };
        // К5 (Д-111): биом суши берётся по климату с вычетом температуры по
        // готовой высоте колонки. Рельеф и вода выше посчитаны по прежнему
        // климату и от этого шага не зависят.
        //
        // К11 / Д-119 (Ш5): «устья у моря» — точки, где C ниже порога океана,
        // а колонка на самом деле суша, — считаются тем же путём, что и
        // остальная суша. Раньше они оставались с океанским поясом и на
        // 3,5 °C теплее соседей, и река в них выходила со строкой океана.
        // Фактическая вода — колонка ниже уровня моря; точка с C ниже порога,
        // но с сушей над водой, остаётся сушей и идёт обычным путём.
        let is_water = is_ocean && class.is_ocean;
        let (climate, class, vanilla) = if is_water {
            (*climate, class, vanilla)
        } else {
            // Высота для вычета — крупная форма: без мелких бугров Д-057 и
            // без вреза русла. Иначе пояс мигал бы на каждом бугре у порога
            // и шёл тёплой лентой по дну горной долины.
            let shape_y = crate::height::sample_micro(
                vanilla.c,
                vanilla.e,
                pv_relief,
                e_km,
                morpho.values[6],
                morpho.values[7],
                0.0,
                0.0,
                shore_dist,
                &self.params,
            )
            .surface_y;
            let cooled = crate::climate::recool(climate, shape_y, &self.params);
            // К11 / Д-119 (Ш5): «устье у моря» — C ниже порога океана, но
            // колонка суша (`class.is_ocean` ложно). Такую точку считаем
            // сушей, иначе она получает океанскую строку T и теплее соседей.
            let recast = classify(&cooled, false, x, z, self.seed, &self.params);
            let (t, h) = crate::vanilla::temperature_humidity(&cooled, &recast, &self.params);
            let base = VanillaSample { t, h, ..base };
            let vanilla = hydrology::apply_fields(base, local_river, &terrain_water, &self.params);
            // Д-118: горные биомы ванили — только на высокой горе (и снежные
            // склоны в снежном поясе); холм остаётся своим биомом.
            let vanilla =
                crate::vanilla_biome::guard_mountain_biomes(vanilla, shape_y, &self.params);
            let class = hydrology::classify_surface(recast, &terrain_water, &cooled, &self.params);
            (cooled, class, vanilla)
        };
        Ok(SurfaceFields {
            climate,
            class,
            vanilla,
            river,
            hydrology: water,
            is_ocean: class.is_ocean,
            pv,
            height,
        })
    }

    /// К4: континентальность в блочной точке без остального климата
    /// (плита + кора, как в `surface_at`, но без луча влаги).
    fn continentalness_at(&self, x: f64, z: f64) -> f64 {
        let point = self.macro_point(x, z);
        match plate_query(point, self.seed, &self.params) {
            Some(plate) => {
                let crust = crust_from_sample(point, &plate, self.seed, &self.params);
                continentalness(crust.e_km, crust.iota_l, &self.params)
            }
            // Без плиты — нейтральная поверхность, как в `surface_at`.
            None => continentalness(0.0, 0.0, &self.params),
        }
    }

    /// К4: континентальность в центре ячейки 4×4 блока, содержащей точку.
    /// Квантование нужно для кэша: пробы берега соседних колонок попадают
    /// в одни ячейки. Ответ зависит только от ячейки, не от запроса.
    fn shore_c_at(&self, x: f64, z: f64) -> f64 {
        let cx = (x / SHORE_CELL).floor() as i64;
        let cz = (z / SHORE_CELL).floor() as i64;
        self.shore_c.get_or_insert((cx, cz), || {
            let px = (cx as f64 + 0.5) * SHORE_CELL;
            let pz = (cz as f64 + 0.5) * SHORE_CELL;
            self.continentalness_at(px, pz)
        })
    }

    /// К4а/часть 2 (Д-110): множитель потенциала рельефа у берега океана —
    /// 0 у воды, 1 дальше `relief.land_fade_blocks` блоков.
    fn shore_fade_from(&self, shore_dist: f64) -> f64 {
        crate::math::smoothstep(0.0, self.params.relief.land_fade_blocks, shore_dist)
    }

    /// То же для точки, где известна только кора (диагностика рельефа).
    fn shore_relief_fade(&self, x: f64, z: f64, crust: &crate::crust::CrustSample) -> f64 {
        let c = continentalness(crust.e_km, crust.iota_l, &self.params);
        self.shore_fade_from(self.shore_distance_blocks(x, z, c))
    }

    /// К4: расстояние до берега океана в блоках; океан даёт 0. Публично
    /// для диагностики песочницы (К4а). Считается по ячейкам кромки, см.
    /// `shore_distance_cells`.
    pub fn shore_distance_blocks(&self, x: f64, z: f64, c: f64) -> f64 {
        if c <= self.params.climate.ocean_c_threshold {
            return 0.0;
        }
        self.shore_distance_cells(x, z)
    }

    /// К4а/часть 2 (Д-110): кромка океана в квадрате `SHORE_BLOCK` блоков —
    /// ячейки 4×4 с океаном (номера внутри квадрата), у которых среди восьми соседей есть суша.
    /// Квадрат, где четыре угла и центр по одну сторону порога, считается
    /// однородным и даёт пустой список: глубь материка и открытый океан не
    /// платят за разбор по ячейкам (залив или островок уже квадрата может
    /// выпасть — тогда у него остаётся прежний берег по окну C).
    fn shore_edge_cells(&self, bx: i64, bz: i64) -> ShoreEdge {
        self.shore_edges.get_or_insert((bx, bz), || {
            let threshold = self.params.climate.ocean_c_threshold;
            let (x0, z0) = (bx as f64 * SHORE_BLOCK, bz as f64 * SHORE_BLOCK);
            let ocean = |x: f64, z: f64| self.shore_c_at(x, z) < threshold;
            let probes = [
                (0.0, 0.0),
                (SHORE_BLOCK, 0.0),
                (0.0, SHORE_BLOCK),
                (SHORE_BLOCK, SHORE_BLOCK),
                (0.5 * SHORE_BLOCK, 0.5 * SHORE_BLOCK),
            ];
            let first = ocean(x0, z0);
            if probes
                .iter()
                .all(|(dx, dz)| ocean(x0 + dx, z0 + dz) == first)
            {
                return Arc::new(Vec::new());
            }
            let n = (SHORE_BLOCK / SHORE_CELL) as i64;
            let centre = |i: i64, j: i64| {
                (
                    x0 + (i as f64 + 0.5) * SHORE_CELL,
                    z0 + (j as f64 + 0.5) * SHORE_CELL,
                )
            };
            let mut cells = Vec::new();
            for j in 0..n {
                for i in 0..n {
                    let (px, pz) = centre(i, j);
                    if !ocean(px, pz) {
                        continue;
                    }
                    let edge = (-1..=1).any(|dj| {
                        (-1..=1).any(|di| {
                            let (qx, qz) = centre(i + di, j + dj);
                            (di != 0 || dj != 0) && !ocean(qx, qz)
                        })
                    });
                    if edge {
                        cells.push([i as u8, j as u8]);
                    }
                }
            }
            Arc::new(cells)
        })
    }

    /// К4а/часть 2 (Д-110): расстояние до берега океана — до ближайшей
    /// ячейки кромки, минус полъячейки. Это настоящее расстояние до одного и
    /// того же набора точек для всех колонок, поэтому оно непрерывно и
    /// меняется не быстрее блока на блок. Прежний марш вдоль градиента давал
    /// его ступенями по 4 блока и терял берег на изгибах — спуск к воде
    /// выходил лестницей. Дальше `reach` блоков берега «нет»: и спуск к
    /// воде, и затухание рельефа там уже выключены.
    fn shore_distance_cells(&self, x: f64, z: f64) -> f64 {
        let reach = self
            .params
            .height
            .shore_width_blocks
            .max(self.params.relief.land_fade_blocks)
            + 0.5 * SHORE_CELL;
        let block = |v: f64| (v / SHORE_BLOCK).floor() as i64;
        let mut best = reach * reach;
        for bz in block(z - reach)..=block(z + reach) {
            for bx in block(x - reach)..=block(x + reach) {
                let (x0, z0) = (bx as f64 * SHORE_BLOCK, bz as f64 * SHORE_BLOCK);
                // Квадрат целиком дальше `reach` — его кромка не нужна.
                let gap_x = (x0 - x).max(x - x0 - SHORE_BLOCK).max(0.0);
                let gap_z = (z0 - z).max(z - z0 - SHORE_BLOCK).max(0.0);
                if gap_x * gap_x + gap_z * gap_z >= best {
                    continue;
                }
                for cell in self.shore_edge_cells(bx, bz).iter() {
                    let dx = x0 + (f64::from(cell[0]) + 0.5) * SHORE_CELL - x;
                    let dz = z0 + (f64::from(cell[1]) + 0.5) * SHORE_CELL - z;
                    best = best.min(dx * dx + dz * dz);
                }
            }
        }
        (best.sqrt() - 0.5 * SHORE_CELL).max(0.0)
    }

    /// Поле глубинной лавы λ (слой `lava`, T53).
    pub fn lava_sample(&self, x: f64, z: f64) -> f64 {
        let point = self.macro_point(x, z);
        match plate_query(point, self.seed, &self.params) {
            Some(sample) => crate::lava::sample(point, &sample, self.seed, &self.params),
            None => self.params.lava.background,
        }
    }

    /// T77: значение поля лавы аквиферов для ячейки игры
    /// `(x div 64, y div 40, z div 64)`. λ берётся в центре ячейки.
    pub fn lava_aquifer(&self, cell_x: i64, cell_y: i64, cell_z: i64) -> f64 {
        let lambda = self.lava_cell(cell_x, cell_z);
        // Д-098: в глубинном слое у стыка лава всегда, без жребия.
        if cell_y * crate::lava::AQUIFER_CELL_Y + crate::lava::AQUIFER_CELL_Y - 1
            <= crate::lava::DEEP_CELL_TOP
            && lambda >= crate::lava::DEEP_LAVA_MIN
        {
            return crate::lava::AQUIFER_LAVA_ON;
        }
        crate::lava::aquifer_value(lambda, [cell_x, cell_y, cell_z], self.seed, &self.params)
    }

    /// Д-098: ячейка 64×64 лежит у стыка плит (λ в её центре не ниже 0,5).
    /// Только здесь остаётся сплошная лава ниже −54.
    pub fn lava_deep(&self, cell_x: i64, cell_z: i64) -> bool {
        self.lava_cell(cell_x, cell_z) >= crate::lava::DEEP_LAVA_MIN
    }

    /// λ в центре ячейки аквифера 64×64 (с кэшем).
    pub fn lava_cell(&self, cell_x: i64, cell_z: i64) -> f64 {
        self.lava_cells.get_or_insert((cell_x, cell_z), || {
            let half = crate::lava::AQUIFER_CELL_XZ as f64 * 0.5;
            let x = (cell_x * crate::lava::AQUIFER_CELL_XZ) as f64 + half;
            let z = (cell_z * crate::lava::AQUIFER_CELL_XZ) as f64 + half;
            self.lava_sample(x, z)
        })
    }

    /// Д-098: в центре ячейки 64×64 суша (поверхность не ниже 60).
    pub fn lava_deep_land(&self, cell_x: i64, cell_z: i64) -> bool {
        self.lava_land.get_or_insert((cell_x, cell_z), || {
            let half = crate::lava::AQUIFER_CELL_XZ as f64 * 0.5;
            let x = (cell_x * crate::lava::AQUIFER_CELL_XZ) as f64 + half;
            let z = (cell_z * crate::lava::AQUIFER_CELL_XZ) as f64 + half;
            let surface = 128.0 * (1.0 + self.field_column(Field::Offset, x, z));
            surface >= crate::lava::DEEP_LAND_MIN_Y
        })
    }

    /// Д-098: затопленность ячейки аквифера с центром в блоке `(x, y, z)`.
    /// `None` — оставить ванильное значение. Ниже −54: у стыка залито до
    /// общего уровня (лава до −54), вдали сухо. От −54 до −41 у стыка на
    /// суше — свой уровень ячейки, который разброс ставит на −54.
    pub fn aquifer_floodedness(&self, x: i64, y: i64, z: i64) -> Option<f64> {
        use crate::lava::{
            AQUIFER_CELL_XZ, DEEP_CELL_TOP, DEEP_LAVA_LEVEL, FLOOD_DRY, FLOOD_FULL, FLOOD_POCKET,
        };
        if y > DEEP_CELL_TOP {
            return None;
        }
        let cell_x = x.div_euclid(AQUIFER_CELL_XZ);
        let cell_z = z.div_euclid(AQUIFER_CELL_XZ);
        let deep = self.lava_deep(cell_x, cell_z);
        if y < DEEP_LAVA_LEVEL {
            return Some(if deep { FLOOD_FULL } else { FLOOD_DRY });
        }
        (deep && self.lava_deep_land(cell_x, cell_z)).then_some(FLOOD_POCKET)
    }

    /// Д-098: исключение аквифера (глубинная тьма) у стыка снимается,
    /// иначе под горами стыка лава пропала бы совсем. `None` — ваниль.
    pub fn aquifer_exclusion(&self, x: i64, y: i64, z: i64) -> Option<f64> {
        use crate::lava::{AQUIFER_CELL_XZ, DEEP_CELL_TOP, EXCLUSION_OFF};
        (y <= DEEP_CELL_TOP
            && self.lava_deep(x.div_euclid(AQUIFER_CELL_XZ), z.div_euclid(AQUIFER_CELL_XZ)))
        .then_some(EXCLUSION_OFF)
    }

    /// Д-098: разброс уровня. Игра спрашивает его в `(x div 16, y div 40,
    /// z div 16)`. В глубинном слое у стыка уровень ровно −54; в остальных
    /// лавовых ячейках ванильный уровень сдвинут на один шаг.
    pub fn aquifer_spread(&self, quad_x: i64, cell_y: i64, quad_z: i64, vanilla: f64) -> f64 {
        use crate::lava::{
            AQUIFER_CELL_Y, AQUIFER_LAVA_OFF, DEEP_CELL_TOP, SPREAD_DEEP, SPREAD_LAVA_SHIFT,
        };
        let cell_x = quad_x.div_euclid(4);
        let cell_z = quad_z.div_euclid(4);
        let top = cell_y * AQUIFER_CELL_Y + AQUIFER_CELL_Y - 1;
        if top <= DEEP_CELL_TOP && self.lava_deep(cell_x, cell_z) {
            return SPREAD_DEEP;
        }
        if top < 0 && self.lava_aquifer(cell_x, cell_y, cell_z) != AQUIFER_LAVA_OFF {
            return vanilla + SPREAD_LAVA_SHIFT;
        }
        vanilla
    }

    /// Д-098: барьер аквифера в блоке `(x, y, z)`. У границы лавовой и
    /// водяной ячеек ставится стенка (в ванили её ставит правило «лава
    /// рядом с водой», которое в наших мирах отключено вместе с общей лавой).
    pub fn aquifer_barrier(&self, x: i64, y: i64, z: i64, vanilla: f64) -> f64 {
        use crate::lava::{
            AQUIFER_CELL_XZ, AQUIFER_CELL_Y, AQUIFER_LAVA_OFF, BARRIER_MARGIN_XZ, BARRIER_MARGIN_Y,
            BARRIER_WALL,
        };
        if y >= 0 {
            return vanilla;
        }
        let side = |v: i64, size: i64, margin: i64| -> i64 {
            let local = v.rem_euclid(size);
            if local < margin {
                -1
            } else if local >= size - margin {
                1
            } else {
                0
            }
        };
        let cell = [
            x.div_euclid(AQUIFER_CELL_XZ),
            y.div_euclid(AQUIFER_CELL_Y),
            z.div_euclid(AQUIFER_CELL_XZ),
        ];
        let step = [
            side(x, AQUIFER_CELL_XZ, BARRIER_MARGIN_XZ),
            side(y, AQUIFER_CELL_Y, BARRIER_MARGIN_Y),
            side(z, AQUIFER_CELL_XZ, BARRIER_MARGIN_XZ),
        ];
        if step == [0, 0, 0] {
            return vanilla;
        }
        // Выше нуля лавы аквиферов нет: соседний слой сверху считается водой.
        let lava =
            |c: [i64; 3]| c[1] < 0 && self.lava_aquifer(c[0], c[1], c[2]) != AQUIFER_LAVA_OFF;
        let own = lava(cell);
        for mask in 1..8_u8 {
            let mut other = cell;
            let mut moved = false;
            for (axis, delta) in step.iter().enumerate() {
                if mask & (1 << axis) != 0 && *delta != 0 {
                    other[axis] += delta;
                    moved = true;
                }
            }
            if moved && lava(other) != own {
                return vanilla.max(BARRIER_WALL);
            }
        }
        vanilla
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
        let key = self.params.config_key;
        let cached = LAST_COARSE.with(|cell| {
            let last = cell.borrow();
            last.as_ref().and_then(|(seed, config, s, x, z, tile)| {
                (*seed == self.seed && *config == key && *s == scale && *x == tx && *z == tz)
                    .then(|| tile.clone())
            })
        });
        if let Some(tile) = cached {
            return tile;
        }
        let tile = self
            .coarse
            .get_or_insert((tx, tz), || self.compute_coarse_tile(tx, tz));
        LAST_COARSE.with(|cell| {
            *cell.borrow_mut() = Some((self.seed, key, scale, tx, tz, tile.clone()));
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
        self.try_sample_with(x, z, true)
    }

    /// R2 (§4): сухая поверхность `S` колонки в блоках — высота до выреза
    /// воды. Самой воде она не нужна, а уровням нужна: зеркало озера и
    /// берега русла считаются от неё, поэтому цикл «уровень → `S`» разорван
    /// флагом `apply_water = false`.
    pub fn dry_surface_at(&self, x: f64, z: f64) -> Result<Option<f64>, RiverNetError> {
        Ok(self
            .try_sample_with(x, z, false)?
            .map(|full| full.world.height.surface_y))
    }

    /// `apply_water = false` — колонка без выреза воды (сухая поверхность `S`).
    fn try_sample_with(
        &self,
        x: f64,
        z: f64,
        apply_water: bool,
    ) -> Result<Option<FullSample>, RiverNetError> {
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
            self.lattice_ray(point, x, z)
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
        let class = self.class_before_height(&climate, is_ocean, x, z);
        let local = if self.params.version.is_water_update() {
            0.0
        } else {
            local_r(x, z, &surface, self.seed, &self.params)
        };
        let fields = self.surface_fields(
            &climate,
            class,
            is_ocean,
            x,
            z,
            local,
            Some((&plates, &crust)),
            apply_water,
        )?;
        let world = WorldSample {
            climate: fields.climate,
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
        // Д-098: признак суши ячеек лавы считается по высоте колонки.
        self.lava_land = Arc::new(ShardedCache::new(
            self.params.lattice.cache_shards.max(1),
            LAVA_CELL_CACHE_CAPACITY,
        ));
        // T79: вместе с лучами сбрасывается и решётка высот; новый номер
        // отвязывает генератор от потоковой памяти тайлов.
        self.ray_field = Arc::new(ShardedCache::new(
            self.params.lattice.cache_shards.max(1),
            crate::ray_field::TILE_CACHE_CAPACITY,
        ));
        self.ray_field_id = RAY_FIELD_IDS.fetch_add(1, Ordering::Relaxed);
        self.ray_field_nodes = Arc::new(std::sync::atomic::AtomicU64::new(0));
        self
    }

    /// T79: сколько узлов решётки высот луча посчитано с создания генератора.
    pub fn ray_field_nodes(&self) -> u64 {
        self.ray_field_nodes.load(Ordering::Relaxed)
    }

    /// T79: решётка луча влаги привязана к блокам и читает высоты с решётки.
    fn ray_field_on(&self) -> bool {
        self.params.version.is_water_relief_dev() && self.params.boundaries.ray_field_l > 0.0
    }

    /// T79: узел решётки высот `(ix, iz)` в шагах `ray_field_l`.
    fn ray_field_node(&self, ix: i64, iz: i64) -> crate::ray_field::RayFieldValue {
        let ((tx, tz), local) = crate::ray_field::locate(ix, iz);
        let slot = (tx.wrapping_mul(3).wrapping_add(tz)).rem_euclid(RAY_TILE_SLOTS as i64) as usize;
        let id = self.ray_field_id;
        // Обычный случай: тайл в памяти потока и узел готов — без замков и
        // без счётчика ссылок.
        let recent = RAY_TILES.with(|tiles| match &tiles.borrow()[slot] {
            Some((owner, x, z, tile)) if *owner == id && *x == tx && *z == tz => {
                tile.get(local).ok_or_else(|| Some(tile.clone()))
            }
            _ => Err(None),
        });
        let tile = match recent {
            Ok(value) => return value,
            Err(Some(tile)) => tile,
            Err(None) => {
                let tile = self
                    .ray_field
                    .get_or_insert((tx, tz), crate::ray_field::RayFieldTile::empty);
                RAY_TILES.with(|tiles| {
                    tiles.borrow_mut()[slot] = Some((id, tx, tz, tile.clone()));
                });
                tile
            }
        };
        tile.node(local, || {
            self.ray_field_nodes.fetch_add(1, Ordering::Relaxed);
            let step = self.params.boundaries.ray_field_l;
            let q = Vec2::new(ix as f64 * step, iz as f64 * step);
            let (wind, phi_deg) = crate::climate::wind_at(q, self.seed, &self.params);
            crate::ray_field::RayFieldValue {
                e_km: self.surface_at(q).e_km,
                wind_x: wind.x,
                wind_z: wind.z,
                phi_deg,
            }
        })
    }

    /// T79: высота в macro-точке луча — сплайн по решётке.
    fn ray_field_height(&self, point: Vec2) -> f64 {
        let step = self.params.boundaries.ray_field_l;
        crate::ray_field::height(point.x / step, point.z / step, |ix, iz| {
            self.ray_field_node(ix, iz)
        })
    }

    /// T79: ветер и широта в macro-точке луча — билинейно по решётке.
    fn ray_field_wind(&self, point: Vec2) -> (Vec2, f64) {
        let step = self.params.boundaries.ray_field_l;
        let v = crate::ray_field::bilinear(point.x / step, point.z / step, |ix, iz| {
            self.ray_field_node(ix, iz)
        });
        (Vec2::new(v.wind_x, v.wind_z), v.phi_deg)
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
        let fast = self.ray_field_on();
        let q = if fast {
            // T79: узел стоит на блочной сетке (32 блока gameplay, 128
            // scientific): редкие колонки дальнего рельефа попадают в узлы.
            let blocks = step * self.params.block.l_blocks;
            self.macro_point(ix as f64 * blocks, iz as f64 * blocks)
        } else {
            Vec2::new(ix as f64 * step, iz as f64 * step)
        };
        let value = crate::climate::pole_sample(q, self.seed, &self.params).map(|pole| {
            let wind = crate::climate::wind(pole.phi_deg, pole.north, pole.east, &self.params);
            let dir = if wind.len() > 1e-9 {
                wind.normalized()
            } else {
                crate::climate::fallback_dir(&self.params)
            };
            if fast {
                // Высота и ветер вдоль луча — с решётки; свой ветер узла и
                // последний шаг колонки остаются точными. Д-025 запрещал
                // брать ветер с грубой решётки (шаг 5 L); на шаге 0.125 L
                // замена точного ветра решёточным зону не меняет (сид 42,
                // gameplay: 0.9783 и так, и так). Совпадение зон определяет
                // высота, поэтому она читается сплайном (Д-064).
                let field = |p: Vec2| SurfaceSample {
                    e_km: self.ray_field_height(p),
                    ..SurfaceSample::default()
                };
                let wind_fn = |p: Vec2| self.ray_field_wind(p);
                return crate::climate::trace_ray_node(
                    q,
                    &self.params,
                    &field,
                    dir,
                    &wind_fn,
                    self.seed,
                );
            }
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
    fn lattice_ray(&self, point: Vec2, x: f64, z: f64) -> Option<crate::climate::RayNode> {
        let step = self.params.boundaries.ray_lattice_l;
        if self.ray_field_on() {
            // T79: решётка в блоках. Узел с нулевым весом не считается:
            // колонка, стоящая в узле, платит за один луч, а не за четыре.
            // `lerp(a, b, 0) == a`, поэтому подстановка ответа не меняет.
            let blocks = step * self.params.block.l_blocks;
            let (gx, gz) = (x / blocks, z / blocks);
            let (ix, iz) = (gx.floor() as i64, gz.floor() as i64);
            let (fx, fz) = (gx - ix as f64, gz - iz as f64);
            let n00 = self.ray_node(ix, iz, step)?;
            let n10 = if fx > 0.0 {
                self.ray_node(ix + 1, iz, step)?
            } else {
                n00
            };
            let (n01, n11) = if fz > 0.0 {
                let n01 = self.ray_node(ix, iz + 1, step)?;
                let n11 = if fx > 0.0 {
                    self.ray_node(ix + 1, iz + 1, step)?
                } else {
                    n01
                };
                (n01, n11)
            } else {
                (n00, n10)
            };
            return Some(crate::climate::RayNode::bilinear(
                &[n00, n10, n01, n11],
                fx,
                fz,
            ));
        }
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
            Field::Pv => {
                // ВО2-1 (§6.7): в зоне calm < 1 ванильный PV −1 — там нет
                // иголок jaggedness над водой.
                if full.world.height.calm < 1.0 {
                    -1.0
                } else {
                    full.world.pv
                }
            }
            Field::Factor => full.world.height.factor,
            Field::Jaggedness => full.world.height.jaggedness,
            Field::Calm => full.world.height.calm,
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

    /// ВО0-2 (PLAN-WATER-UPDATE §6.1): сухая поверхность колонки `S0` —
    /// точный путь по той же цепочке, что `surface_fields`, но **до**
    /// гидрологии: без `hydrology::apply_fields`, без силы `river` и без
    /// `e_valley_km`. Поэтому `S0` не зависит от воды: её уровни потом
    /// считаются уже от него.
    ///
    /// `None` — версия без водного обновления или точка без климата/плиты.
    pub fn s0_at(&self, x: f64, z: f64) -> Option<S0Sample> {
        let probe = self.w_probe(x, z)?;
        // Разность двух вызовов, а не повтор формулы: вклад микробугров уже
        // взвешен берегом (`coast`), как в игре, и `S0` совпадает с полной
        // поверхностью побитово.
        let full_h = crate::height::sample_micro(
            probe.base_c,
            probe.base_e,
            probe.pv_relief,
            probe.e_km,
            probe.cold,
            probe.warm,
            probe.micro,
            1.0,
            probe.shore_dist,
            &self.params,
        );
        Some(S0Sample {
            macro_y: probe.macro_y,
            micro_dy: full_h.surface_y - probe.macro_y,
        })
    }

    /// ВО0-2: быстрый `S0` для узлов решётки стока (§6.2).
    ///
    /// Пока это тот же точный путь: выбор приближения (путь А на 0,25 L или
    /// «Б-лайт» на 0,5 L) делает разведка ВО0-5 по цене сети плиты и
    /// точности. Поэтому тест точности `s0_node` против `s0_at` сейчас
    /// нулевой, а числа ВО0-5 меряют уже эту функцию.
    pub fn s0_node(&self, x: f64, z: f64) -> Option<S0Sample> {
        self.s0_at(x, z)
    }

    /// R2 (Ф6-В1): вода колонки для заливки в игре. Уровень и дно берутся из
    /// того же `cached_full`, что и поля, поэтому вода и рельеф согласованы.
    /// Вода ниже уровня моря сюда не входит: её ставит сама игра.
    pub fn water_column(&self, x: f64, z: f64) -> WaterColumn {
        let Ok(Some(full)) = self.cached_full(x, z) else {
            return WaterColumn::default();
        };
        let height = &full.world.height;
        // Поверхность нужна и без воды: по ней проверяется инвариант «берег
        // не ниже зеркала» (§4, п. 2) и считается «сухое русло» в CI.
        let surface = height.surface_y.round() as i32;
        let kind = full.world.hydrology.kind;
        let body = body_from_kind(kind);
        // ВО2-2: класс потока и материалы заполняются профилем W/Java на
        // этапах ВО3–ВО5; здесь — значения по умолчанию.
        let flow = crate::water_profile::FlowClass::Calm;
        if height.water_strength <= 0.0 {
            return WaterColumn {
                surface,
                kind,
                body,
                flow,
                ..WaterColumn::default()
            };
        }
        let level = height.water_y.round();
        // Русло у моря (запасной вырез R2): воду до y = 62 ставит игра, но
        // 3D-шум поднимает дно русла выше 62, и вода остаётся лужами (игровой
        // тест 2026-11-19, (1421, -1276)). Поэтому такое русло тоже отдаётся
        // в игру с уровнем 63 — Java прорезает его по настоящим блокам. Выше
        // моря эта вода не встаёт, инвариантов R2 (§4) она не касается.
        if level.is_finite()
            && level <= crate::height::SEA_LEVEL
            && kind == crate::hydrology::SurfaceKind::River
            && full.world.hydrology.channels.core > 0.0
        {
            return WaterColumn {
                level: crate::height::SEA_LEVEL as i32,
                bed: height.bed_y.round() as i32,
                surface,
                kind,
                body,
                flow,
                bed_mat: 0,
                shore_mat: 0,
            };
        }
        // NaN и «не выше моря» — одно и то же: колонка остаётся сушей (clippy:
        // neg_cmp_op_on_partial_ord не даёт писать `!(level > SEA_LEVEL)`).
        if !level.is_finite() || level <= crate::height::SEA_LEVEL {
            return WaterColumn {
                surface,
                kind,
                body,
                flow,
                ..WaterColumn::default()
            };
        }
        WaterColumn {
            level: level as i32,
            bed: height.bed_y.round() as i32,
            surface,
            kind,
            body,
            flow,
            bed_mat: 0,
            shore_mat: 0,
        }
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

    /// ВО1-4: профиль `W` вниз по течению не поднимается. Подпор (п. 4)
    /// поднимает только верховья, а выравнивание притока — только низовья,
    /// поэтому `(up, down)` каждого сегмента остаётся убывающим.
    #[test]
    fn w_levels_never_rise_downstream() {
        let mut reaches = 0;
        for seed in [42, 7, 1, -1] {
            for mode in Mode::ALL {
                let generator = Generator::new(seed, mode, GenVersion::V1DevW).unwrap();
                let point = generator.macro_point(0.0, 0.0);
                for plate in
                    crate::plates::plates_near(point, 10.0, generator.seed(), &generator.params)
                        .into_iter()
                        .take(1)
                {
                    let Ok(net) = generator.drainage_network(&plate) else {
                        continue;
                    };
                    let levels = generator.w_plate_levels(&plate, &net).unwrap();
                    for reach in 0..net.channels().reaches().len() {
                        let Ok(profile) = generator.w_reach_profile_inner(&levels, &net, reach)
                        else {
                            continue;
                        };
                        for &(up, down) in profile.levels.iter().flatten() {
                            assert!(
                                down <= up + 1e-9,
                                "уровень вырос вниз по течению: {up} → {down}, звено {reach}, сид {seed}, {mode:?}"
                            );
                        }
                        reaches += 1;
                    }
                }
            }
        }
        assert!(reaches > 0, "ни одного профиля W не построено");
    }

    /// T79: список плит из блоков совпадает с прямым расчётом Д-061, в том
    /// числе для окон через границы блоков и в отрицательных координатах.
    #[test]
    fn owner_blocks_match_direct_window() {
        let generator = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let windows = [
            ((0, 0), (27, 27)),
            ((-13, -13), (14, 14)),
            ((-40, 5), (-13, 32)),
            ((1470, 90), (1497, 117)),
            ((15, 15), (16, 16)),
            ((-17, -1), (-16, 0)),
            ((300, -700), (420, -650)),
        ];
        for (lo, hi) in windows {
            let direct =
                crate::drainage::owners_in_window(lo, hi, generator.seed, &generator.params);
            assert_eq!(
                generator.owners_in_window(lo, hi),
                direct,
                "окно {lo:?}…{hi:?}"
            );
            assert!(!direct.is_empty());
        }
    }

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
                Field::Pv => {
                    if full.world.height.calm < 1.0 {
                        -1.0
                    } else {
                        full.world.pv
                    }
                }
                Field::Offset => full.world.height.offset,
                Field::Factor => full.world.height.factor,
                Field::Jaggedness => full.world.height.jaggedness,
                Field::Calm => full.world.height.calm,
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
        assert_eq!(g.params.boundaries.ray_field_l, 0.0);
        for (x, z) in points(8) {
            let fast = g.sample(x, z).unwrap();
            let reference = g.sample_reference(x, z).unwrap();
            assert_eq!(
                fast.climate.precip.mu.to_bits(),
                reference.climate.precip.mu.to_bits()
            );
        }
        assert_eq!(g.ray_nodes(), 0);
        assert_eq!(g.ray_field_nodes(), 0);
    }

    /// T79: решётка луча стоит на блочной сетке (32 блока в gameplay).
    /// Колонка в узле платит за один луч, на ребре — за два, внутри клетки —
    /// за четыре: редкие колонки дальнего рельефа попадают в узлы.
    #[test]
    fn column_on_a_lattice_node_traces_one_ray() {
        let fresh = || Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let blocks = fresh().params.boundaries.ray_lattice_l * Mode::Gameplay.l_blocks();
        assert_eq!(blocks, 32.0);
        for (x, z, rays) in [
            (640.0, -1280.0, 1),
            (640.0, -1279.0, 2),
            (641.0, -1280.0, 2),
            (641.0, -1279.0, 4),
        ] {
            let g = fresh();
            g.sample(x, z).unwrap();
            assert_eq!(g.ray_nodes(), rays, "колонка ({x}, {z})");
            assert!(g.ray_field_nodes() > 0);
        }
    }

    /// T79: узел решётки высот хранит точную высоту, ветер и широту своей
    /// точки и считается один раз.
    #[test]
    fn ray_field_node_holds_exact_height_and_wind() {
        for mode in Mode::ALL {
            let g = Generator::new(42, mode, GenVersion::V1Dev).unwrap();
            let step = g.params.boundaries.ray_field_l;
            assert!(step > 0.0);
            let nodes = [(0, 0), (17, -5), (-33, 40), (-1, -1), (16, 16)];
            for (ix, iz) in nodes {
                let q = Vec2::new(ix as f64 * step, iz as f64 * step);
                let node = g.ray_field_node(ix, iz);
                let (wind, phi_deg) = crate::climate::wind_at(q, g.seed, &g.params);
                assert_eq!(node.e_km.to_bits(), g.surface_at(q).e_km.to_bits());
                assert_eq!(node.wind_x.to_bits(), wind.x.to_bits());
                assert_eq!(node.wind_z.to_bits(), wind.z.to_bits());
                assert_eq!(node.phi_deg.to_bits(), phi_deg.to_bits());
                assert_eq!(g.ray_field_node(ix, iz), node);
            }
            assert_eq!(g.ray_field_nodes(), nodes.len() as u64);
            // Сплайн в узле отдаёт высоту узла (и читает его соседей).
            for (ix, iz) in nodes {
                let q = Vec2::new(ix as f64 * step, iz as f64 * step);
                let node = g.ray_field_node(ix, iz);
                assert_eq!(g.ray_field_height(q).to_bits(), node.e_km.to_bits());
            }
            assert!(g.ray_field_nodes() > nodes.len() as u64);
        }
    }

    /// T79: два мира в одном потоке не читают тайлы друг друга, а сброс
    /// решётки отвязывает генератор от потоковой памяти тайлов (Р14).
    #[test]
    fn ray_field_tiles_are_not_shared_between_worlds() {
        let a = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let b = Generator::new(43, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let va = a.ray_field_node(3, 3);
        let vb = b.ray_field_node(3, 3);
        assert_ne!(va, vb);
        assert_eq!(a.ray_field_node(3, 3), va);
        assert_eq!(b.ray_field_node(3, 3), vb);
        let lattice = a.params.boundaries.ray_lattice_l;
        let reset = a.clone().with_ray_lattice(lattice);
        assert_eq!(reset.ray_field_nodes(), 0);
        assert_eq!(reset.ray_field_node(3, 3), va);
        assert_eq!(reset.ray_field_nodes(), 1);
        assert_eq!(a.ray_field_nodes(), 1);
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

#[cfg(test)]
mod net_owner_tests {
    use super::Generator;
    use crate::mode::Mode;
    use crate::plates::plates_near;
    use crate::version::GenVersion;

    /// Д-061: сеть плиты, не попавшей в список `net_owners`, в точке молчит:
    /// нет русла, долины, озера и глубины впадины. Значит, пропуск такой сети
    /// не меняет ответ, а только избавляет от её постройки.
    #[test]
    fn skipped_river_nets_are_silent_at_the_point() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1Dev).unwrap();
        let radius = crate::channel::max_query_radius_l(g.params());
        let mut skipped = 0_usize;
        let mut kept = 0_usize;
        for i in -4..=4 {
            for j in -4..=4 {
                let (x, z) = (f64::from(i) * 71.5 + 3.5, f64::from(j) * 67.5 - 9.5);
                let point = g.macro_point(x, z);
                let owners = g.net_owners(point);
                assert!(owners.windows(2).all(|pair| pair[0] < pair[1]));
                for plate in plates_near(point, radius, g.seed(), g.params()) {
                    if owners.binary_search(&plate.id).is_ok() {
                        kept += 1;
                        continue;
                    }
                    skipped += 1;
                    let net = g.drainage_network(&plate).unwrap();
                    let channel = net.channels().query(point);
                    assert!(
                        channel.nearest.is_none()
                            && channel.core == 0.0
                            && channel.bank == 0.0
                            && channel.valley == 0.0
                            && channel.delta == 0.0
                            && channel.gallery == 0.0,
                        "русло пропущенной плиты {} в ({x}, {z}): {channel:?}",
                        plate.id
                    );
                    let lake = net.lakes().query(point);
                    assert!(
                        lake.nearest.is_none()
                            && lake.water == 0.0
                            && lake.shore == 0.0
                            && lake.basin == 0.0
                            && lake.sea_depth_km == 0.0,
                        "озеро пропущенной плиты {} в ({x}, {z}): {lake:?}",
                        plate.id
                    );
                    assert_eq!(net.depression_depth_km(point), 0.0);
                }
            }
        }
        // Отбор действительно работает: у точки остаётся 1–3 сети из 13.
        assert!(kept >= 81, "оставлено {kept}");
        assert!(skipped > 5 * kept, "пропущено {skipped}, оставлено {kept}");
    }

    /// Список зависит только от тайла: порядок запросов и вытеснение из
    /// кэша его не меняют.
    #[test]
    fn owner_list_does_not_depend_on_query_order() {
        let forward = Generator::new(7, Mode::Scientific, GenVersion::V1Dev).unwrap();
        let backward = Generator::new(7, Mode::Scientific, GenVersion::V1Dev).unwrap();
        let points: Vec<_> = (0..40)
            .map(|k| forward.macro_point(f64::from(k) * 913.0 - 15000.0, f64::from(k * k) * 37.0))
            .collect();
        let a: Vec<_> = points.iter().map(|p| forward.net_owners(*p)).collect();
        let mut b: Vec<_> = points
            .iter()
            .rev()
            .map(|p| backward.net_owners(*p))
            .collect();
        b.reverse();
        assert_eq!(a, b);
        assert!(a.iter().all(|owners| !owners.is_empty()));
    }
}
