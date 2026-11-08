//! Все числа плана в одном месте (план 9.5, приложение D).
//!
//! В формулах не должно быть литералов: любое число живёт здесь. Набор
//! `defaults(version, mode)` для выпущенной версии замораживается навсегда.
//! `params_hash()` пишется в лог мода и в подпись картинок песочницы.

use std::fmt::Write as _;

use crate::hash::fnv1a64;
use crate::mode::{BlockParams, Mode};
use crate::version::GenVersion;

#[derive(Clone, Debug, PartialEq)]
pub struct PlatesParams {
    pub cell_g_l: f64,
    pub inset_fraction: f64,
    pub empty_probability: f64,
    pub double_probability: f64,
    pub min_seed_distance_fraction: f64,
    pub seed_attempts: usize,
    pub center_offset_fraction: f64,
    pub continent_probability: f64,
    pub continent_radius_min_l: f64,
    pub continent_radius_max_l: f64,
    /// Показатель степени для разброса размеров материков (Д-021).
    /// При 1 — равномерно, при > 1 — много мелких и единицы крупных.
    pub continent_radius_skew: f64,
    pub aspect_min: f64,
    pub aspect_max: f64,
    pub speed_min: f64,
    pub speed_max: f64,
    pub omega_max: f64,
    pub warp_lambda0_l: f64,
    pub warp_amplitude_l: f64,
    pub warp_octaves: usize,
    pub warp_gain: f64,
    pub query_radius_cells: i64,
    pub triple_weight_l: f64,
    pub boundary_conv_threshold: f64,
    pub boundary_shear_threshold: f64,
    pub boundary_width: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrustParams {
    pub shelf_depth_km: f64,
    pub abyss_depth_km: f64,
    pub slope_start_l: f64,
    pub slope_end_l: f64,
    pub continent_height_km: f64,
    pub h_col_km: f64,
    pub w_col_l: f64,
    pub h_trench_km: f64,
    pub d_trench_l: f64,
    pub w_trench_l: f64,
    pub h_arc_km: f64,
    pub d_arc_l: f64,
    pub w_arc_l: f64,
    pub h_island_km: f64,
    pub d_island_l: f64,
    pub w_island_l: f64,
    pub h_ridge_km: f64,
    pub w_ridge_l: f64,
    pub h_axial_km: f64,
    pub w_axial_l: f64,
    pub h_rift_km: f64,
    pub w_rift_l: f64,
    pub h_shoulder_km: f64,
    pub ocean_kappa_l: f64,
    /// Радиус окна поиска центров материков в ячейках плит (Д-012).
    pub continent_query_radius_cells: i64,
    pub kappa_noise_amplitude_l: f64,
    pub kappa_noise_lambda_l: f64,
    pub kappa_noise_octaves: usize,
    pub kappa_noise_gain: f64,
    pub blend_width_l: f64,
    pub boundary_mix_factor: f64,
    pub shelf_start_l: f64,
    pub shelf_end_l: f64,
    pub continent_scale_l: f64,
    pub slope_drop_km: f64,
    pub plateau_factor: f64,
    pub island_noise_lambda_l: f64,
    pub island_noise_base: f64,
    pub island_noise_amp: f64,
    pub rift_shoulder_offset: f64,
    pub u6_offset_l: f64,
    pub u6_width_l: f64,
    pub u6_drop_km: f64,
    pub u7_amplitude_km: f64,
    pub u7_width_l: f64,
    pub u7_noise_lambda_l: f64,
    pub mountain_edge_l: f64,
    pub mountain_scale_km: f64,
    pub mountain_transform_factor: f64,
    pub detail_land_km: f64,
    pub detail_ocean_km: f64,
    pub detail_mountain_km: f64,
    pub detail_land_lambda_l: f64,
    pub detail_ocean_lambda_l: f64,
    pub detail_mountain_lambda_l: f64,
    pub detail_octaves: usize,
    pub detail_gain: f64,
    pub iota_height_factor: f64,
    pub iota_max_l: f64,
    pub hotspot_inset: f64,
    pub hotspot_chain_count: usize,
    pub hotspot_chain_step_l: f64,
    pub hotspot_chain_decay: f64,
    pub hotspot_radius_growth: f64,
    pub hotspot_continental_factor: f64,
    pub hotspot_consider_radius_l: f64,
    pub hotspot_cell_l: f64,
    pub hotspot_probability: f64,
    pub h_hotspot_km: f64,
    pub r_hotspot_l: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PolesParams {
    pub cell_g_p_l: f64,
    pub inset_fraction: f64,
    pub warp_lambda0_l: f64,
    pub warp_amplitude_l: f64,
    pub warp_octaves: usize,
    pub warp_gain: f64,
    pub query_radius_cells: i64,
    pub phi_noise_degrees: f64,
    /// Масштаб и октавы дрожания псевдошироты (Д-021): рваные кольца.
    /// λ₀ обязан быть много крупнее coarse-решётки климата, иначе сглаживание
    /// узлов гасит дрожь только в быстром пути и зоны перестают совпадать.
    pub phi_noise_lambda_l: f64,
    pub phi_noise_octaves: usize,
    pub phi_noise_gain: f64,
}

impl CrustParams {
    /// Дополнительные числа слоя L2: участвуют в `params_hash` (9.5).
    pub fn extra_text(&self) -> String {
        let mut out = String::new();
        let w = &mut out;
        writeln!(w, "crust.ocean_kappa_l={v}", v = self.ocean_kappa_l).unwrap();
        writeln!(
            w,
            "crust.continent_query_radius_cells={v}",
            v = self.continent_query_radius_cells
        )
        .unwrap();
        writeln!(
            w,
            "crust.kappa_noise_amplitude_l={v}",
            v = self.kappa_noise_amplitude_l
        )
        .unwrap();
        writeln!(
            w,
            "crust.kappa_noise_lambda_l={v}",
            v = self.kappa_noise_lambda_l
        )
        .unwrap();
        writeln!(
            w,
            "crust.kappa_noise_octaves={v}",
            v = self.kappa_noise_octaves
        )
        .unwrap();
        writeln!(w, "crust.kappa_noise_gain={v}", v = self.kappa_noise_gain).unwrap();
        writeln!(w, "crust.blend_width_l={v}", v = self.blend_width_l).unwrap();
        writeln!(
            w,
            "crust.boundary_mix_factor={v}",
            v = self.boundary_mix_factor
        )
        .unwrap();
        writeln!(w, "crust.shelf_start_l={v}", v = self.shelf_start_l).unwrap();
        writeln!(w, "crust.shelf_end_l={v}", v = self.shelf_end_l).unwrap();
        writeln!(w, "crust.continent_scale_l={v}", v = self.continent_scale_l).unwrap();
        writeln!(w, "crust.slope_drop_km={v}", v = self.slope_drop_km).unwrap();
        writeln!(w, "crust.plateau_factor={v}", v = self.plateau_factor).unwrap();
        writeln!(
            w,
            "crust.island_noise_lambda_l={v}",
            v = self.island_noise_lambda_l
        )
        .unwrap();
        writeln!(w, "crust.island_noise_base={v}", v = self.island_noise_base).unwrap();
        writeln!(w, "crust.island_noise_amp={v}", v = self.island_noise_amp).unwrap();
        writeln!(
            w,
            "crust.rift_shoulder_offset={v}",
            v = self.rift_shoulder_offset
        )
        .unwrap();
        writeln!(w, "crust.u6_offset_l={v}", v = self.u6_offset_l).unwrap();
        writeln!(w, "crust.u6_width_l={v}", v = self.u6_width_l).unwrap();
        writeln!(w, "crust.u6_drop_km={v}", v = self.u6_drop_km).unwrap();
        writeln!(w, "crust.u7_amplitude_km={v}", v = self.u7_amplitude_km).unwrap();
        writeln!(w, "crust.u7_width_l={v}", v = self.u7_width_l).unwrap();
        writeln!(w, "crust.u7_noise_lambda_l={v}", v = self.u7_noise_lambda_l).unwrap();
        writeln!(w, "crust.mountain_edge_l={v}", v = self.mountain_edge_l).unwrap();
        writeln!(w, "crust.mountain_scale_km={v}", v = self.mountain_scale_km).unwrap();
        writeln!(
            w,
            "crust.mountain_transform_factor={v}",
            v = self.mountain_transform_factor
        )
        .unwrap();
        writeln!(w, "crust.detail_land_km={v}", v = self.detail_land_km).unwrap();
        writeln!(w, "crust.detail_ocean_km={v}", v = self.detail_ocean_km).unwrap();
        writeln!(
            w,
            "crust.detail_mountain_km={v}",
            v = self.detail_mountain_km
        )
        .unwrap();
        writeln!(
            w,
            "crust.detail_land_lambda_l={v}",
            v = self.detail_land_lambda_l
        )
        .unwrap();
        writeln!(
            w,
            "crust.detail_ocean_lambda_l={v}",
            v = self.detail_ocean_lambda_l
        )
        .unwrap();
        writeln!(
            w,
            "crust.detail_mountain_lambda_l={v}",
            v = self.detail_mountain_lambda_l
        )
        .unwrap();
        writeln!(w, "crust.detail_octaves={v}", v = self.detail_octaves).unwrap();
        writeln!(w, "crust.detail_gain={v}", v = self.detail_gain).unwrap();
        writeln!(
            w,
            "crust.iota_height_factor={v}",
            v = self.iota_height_factor
        )
        .unwrap();
        writeln!(w, "crust.iota_max_l={v}", v = self.iota_max_l).unwrap();
        writeln!(w, "crust.hotspot_inset={v}", v = self.hotspot_inset).unwrap();
        writeln!(
            w,
            "crust.hotspot_chain_count={v}",
            v = self.hotspot_chain_count
        )
        .unwrap();
        writeln!(
            w,
            "crust.hotspot_chain_step_l={v}",
            v = self.hotspot_chain_step_l
        )
        .unwrap();
        writeln!(
            w,
            "crust.hotspot_chain_decay={v}",
            v = self.hotspot_chain_decay
        )
        .unwrap();
        writeln!(
            w,
            "crust.hotspot_radius_growth={v}",
            v = self.hotspot_radius_growth
        )
        .unwrap();
        writeln!(
            w,
            "crust.hotspot_continental_factor={v}",
            v = self.hotspot_continental_factor
        )
        .unwrap();
        writeln!(
            w,
            "crust.hotspot_consider_radius_l={v}",
            v = self.hotspot_consider_radius_l
        )
        .unwrap();
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindParams {
    pub strength_phi_deg: [f64; 8],
    pub strength_s: [f64; 8],
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrecipParams {
    pub ray_steps: usize,
    pub ray_step_l: f64,
    pub ocean_length_l: f64,
    pub land_length_l: f64,
    pub orographic_height_km: f64,
    pub itcz_precip_mm: f64,
    pub itcz_width_deg: f64,
    pub front_precip_mm: f64,
    pub front_center_deg: f64,
    pub front_width_deg: f64,
    pub local_fraction_base: f64,
    pub advection_mm: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiversParams {
    pub source_cell_l: f64,
    pub source_probability: f64,
    pub source_min_mountain: f64,
    pub source_min_height_km: f64,
    pub source_min_precip_mm: f64,
    pub source_window_cells: i64,
    pub trace_step_l: f64,
    pub trace_max_steps: usize,
    pub trace_min_length_l: f64,
    pub trace_gradient_min: f64,
    pub trace_flat_steps: usize,
    pub meander_turn_rad: f64,
    pub gradient_step_l: f64,
    pub foothill_sigma_factor: f64,
    pub half_width_growth: f64,
    pub half_width_length_l: f64,
}

/// T110/T111: сеть стока и климатический расход. Используется только
/// v1-dev; ключи v0 не получают новых строк.
#[derive(Clone, Debug, PartialEq)]
pub struct DrainageParams {
    pub step_l: f64,
    pub jitter_fraction: f64,
    pub fill_epsilon_km: f64,
    pub depression_min_depth_km: f64,
    pub outlet_min_weight: f64,
    pub anchor_search_radius_cells: i64,
    /// T111: шаг решётки P/PET в L; в узлы — билинейно.
    pub climate_step_l: f64,
    /// T111: коэффициент перед PET в стоке суши P - k_et·PET.
    pub et_factor: f64,
}

/// T112: геометрия русел поверх Q. Числа блочные, поэтому режимы различаются;
/// сама сеть стока остаётся в L. Используется только v1-dev.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelParams {
    pub q_min_mm_l2: f64,
    pub core_min_blocks: f64,
    pub core_max_blocks: f64,
    pub depth0_blocks: f64,
    pub meander_plain: f64,
    pub meander_mountain: f64,
    pub valley_plain: f64,
    pub valley_mountain: f64,
    pub plain_slope_max: f64,
    /// T114: дельта на пологом берегу при большом расходе.
    pub delta_q_min_mm_l2: f64,
    pub delta_slope_max: f64,
    pub delta_length_min_l: f64,
    pub delta_length_max_l: f64,
}

/// T114: озёра и котловины поверх впадин T110 и баланса T111.
/// Макропараметры одинаковы в обоих режимах; высокие озёра требуют R2,
/// но не отсекаются потолком высоты (решение автора в §5.6).
#[derive(Clone, Debug, PartialEq)]
pub struct LakeParams {
    pub inland_sea_area_l2: f64,
    pub swamp_max_depth_km: f64,
    pub swamp_min_p_pet: f64,
    pub frozen_t_min_c: f64,
    /// Полуширина сглаживания берега в долях шага решётки стока.
    pub shore_fraction: f64,
}

/// T115: перевод гидрологии в ванильные C/E/W без собственного PV (R1).
/// Высокие озёра выше порога R1 получают только сушу/болото до T143/T145.
#[derive(Clone, Debug, PartialEq)]
pub struct HydrologyParams {
    pub bank_fraction: f64,
    /// Минимальная ширина сглаживания C/E/W вокруг русла/берега, L.
    /// 0.48 L = 120 блоков gameplay при SEAM_LIMIT=0.02; macro-масштаб
    /// един, поэтому научный режим не меняет геометрию сетей.
    pub field_fade_l: f64,
    pub floodplain_e: f64,
    pub swamp_e: f64,
    pub dry_basin_e: f64,
    pub estuary_c: f64,
    pub estuary_elevation_km: f64,
    pub inland_sea_c: f64,
    pub r1_max_level_km: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LavaParams {
    pub width_blocks: f64,
    pub background: f64,
    pub base_factor: f64,
    pub boundary_factor: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnParams {
    pub candidates: usize,
    pub radius_l: f64,
    pub fallback_grid: usize,
    pub min_e_km: f64,
    pub max_e_km: f64,
    pub min_kappa_l: f64,
    pub max_mountain: f64,
    pub min_phi_deg: f64,
    pub max_phi_deg: f64,
    pub min_ratio: f64,
    pub max_ratio: f64,
    pub fixed_mu: f64,
    pub fixed_wind: f64,
    pub search_radius_blocks: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LatticeParams {
    pub fine_divisor: f64,
    pub coarse_divisor: f64,
    pub tile_nodes: usize,
    pub halo_nodes: usize,
    pub cache_shards: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClimateParams {
    pub t_equator_c: f64,
    pub t_pole_drop_c: f64,
    pub t_lat_exponent: f64,
    pub lapse_c_per_km: f64,
    pub cont_tropical_c: f64,
    pub cont_polar_c: f64,
    pub amp_base_c: f64,
    pub amp_lat_c: f64,
    pub amp_sea_factor: f64,
    pub amp_land_factor: f64,
    pub sst_min_c: f64,
    pub bio_min_c: f64,
    pub bio_max_c: f64,
    pub coldest_cos: f64,
    pub trade_n: f64,
    pub trade_e: f64,
    pub west_n: f64,
    pub west_e: f64,
    pub polar_n: f64,
    pub polar_e: f64,
    pub belt_trade_deg: [f64; 2],
    pub belt_west_deg: [f64; 2],
    pub belt_polar_deg: [f64; 2],
    pub wind_fallback_dir: [f64; 2],
    pub w_sat_base: f64,
    pub w_sat_per_c: f64,
    pub w_sat_min: f64,
    pub w_sat_max: f64,
    pub w_land_start: f64,
    pub cont_decay_l: f64,
    pub c_iso_end_l: f64,
    pub c_ray_wind_lo: f64,
    pub c_ray_wind_hi: f64,
    pub pet_per_bio_c: f64,
    pub p_min_mm: f64,
    pub slope_probe_l: f64,
    pub uplift_base: f64,
    pub uplift_per_slope: f64,
    pub uplift_slope_max: f64,
    pub advection_wind_min: f64,
    pub local_fraction_equator: f64,
    pub local_fraction_width_deg: f64,
    pub belt_edges_c: [f64; 6],
    pub province_edges_r: [f64; 5],
    pub winter_boreal_c: f64,
    pub winter_temperate_c: f64,
    pub cold_desert_sand: bool,
    /// Песок допустим только там, где самый холодный месяц не ниже этого.
    pub cold_desert_winter_c: f64,
    pub jitter_lambda_blocks: f64,
    pub jitter_octaves: usize,
    pub jitter_gain: f64,
    pub sst_bands_c: [f64; 4],
    pub c_sea_e_km: [f64; 7],
    pub c_sea: [f64; 7],
    pub c_land_iota_l: [f64; 7],
    pub c_land: [f64; 7],
    pub c_coast_e_km: f64,
    pub c_coast: f64,
    pub ocean_c_threshold: f64,
    pub e_base: f64,
    pub e_noise: f64,
    pub e_mnt: f64,
    pub e_mnt_noise: f64,
    pub e_plateau: f64,
    pub m_e_lo: f64,
    pub m_e_hi: f64,
    pub w_plain_base: f64,
    pub w_plain_noise: f64,
    pub w_mnt_base: f64,
    pub w_mnt_noise: f64,
    pub w_plateau: f64,
    pub w_max: f64,
    pub w_sign_scale: f64,
    /// T102: нижняя граница |∇f|·w_s, только v1-dev.
    pub w_sign_epsilon: f64,
    pub t_centers: [f64; 5],
    pub h_centers: [f64; 5],
    pub h4_base: f64,
    pub h4_gain: f64,
}

impl ClimateParams {
    /// Все числа слоёв L3–L6 участвуют в `params_hash` (9.5).
    pub fn extra_text(&self) -> String {
        let mut out = String::new();
        let w = &mut out;
        writeln!(w, "climate.t_equator_c={:?}", self.t_equator_c).unwrap();
        writeln!(w, "climate.t_pole_drop_c={:?}", self.t_pole_drop_c).unwrap();
        writeln!(w, "climate.t_lat_exponent={:?}", self.t_lat_exponent).unwrap();
        writeln!(w, "climate.lapse_c_per_km={:?}", self.lapse_c_per_km).unwrap();
        writeln!(w, "climate.cont_tropical_c={:?}", self.cont_tropical_c).unwrap();
        writeln!(w, "climate.cont_polar_c={:?}", self.cont_polar_c).unwrap();
        writeln!(w, "climate.amp_base_c={:?}", self.amp_base_c).unwrap();
        writeln!(w, "climate.amp_lat_c={:?}", self.amp_lat_c).unwrap();
        writeln!(w, "climate.amp_sea_factor={:?}", self.amp_sea_factor).unwrap();
        writeln!(w, "climate.amp_land_factor={:?}", self.amp_land_factor).unwrap();
        writeln!(w, "climate.sst_min_c={:?}", self.sst_min_c).unwrap();
        writeln!(w, "climate.bio_min_c={:?}", self.bio_min_c).unwrap();
        writeln!(w, "climate.bio_max_c={:?}", self.bio_max_c).unwrap();
        writeln!(w, "climate.coldest_cos={:?}", self.coldest_cos).unwrap();
        writeln!(w, "climate.trade_n={:?}", self.trade_n).unwrap();
        writeln!(w, "climate.trade_e={:?}", self.trade_e).unwrap();
        writeln!(w, "climate.west_n={:?}", self.west_n).unwrap();
        writeln!(w, "climate.west_e={:?}", self.west_e).unwrap();
        writeln!(w, "climate.polar_n={:?}", self.polar_n).unwrap();
        writeln!(w, "climate.polar_e={:?}", self.polar_e).unwrap();
        writeln!(w, "climate.belt_trade_deg={:?}", self.belt_trade_deg).unwrap();
        writeln!(w, "climate.belt_west_deg={:?}", self.belt_west_deg).unwrap();
        writeln!(w, "climate.belt_polar_deg={:?}", self.belt_polar_deg).unwrap();
        writeln!(w, "climate.wind_fallback_dir={:?}", self.wind_fallback_dir).unwrap();
        writeln!(w, "climate.w_sat_base={:?}", self.w_sat_base).unwrap();
        writeln!(w, "climate.w_sat_per_c={:?}", self.w_sat_per_c).unwrap();
        writeln!(w, "climate.w_sat_min={:?}", self.w_sat_min).unwrap();
        writeln!(w, "climate.w_sat_max={:?}", self.w_sat_max).unwrap();
        writeln!(w, "climate.w_land_start={:?}", self.w_land_start).unwrap();
        writeln!(w, "climate.cont_decay_l={:?}", self.cont_decay_l).unwrap();
        writeln!(w, "climate.c_iso_end_l={:?}", self.c_iso_end_l).unwrap();
        writeln!(w, "climate.c_ray_wind_lo={:?}", self.c_ray_wind_lo).unwrap();
        writeln!(w, "climate.c_ray_wind_hi={:?}", self.c_ray_wind_hi).unwrap();
        writeln!(w, "climate.pet_per_bio_c={:?}", self.pet_per_bio_c).unwrap();
        writeln!(w, "climate.p_min_mm={:?}", self.p_min_mm).unwrap();
        writeln!(w, "climate.slope_probe_l={:?}", self.slope_probe_l).unwrap();
        writeln!(w, "climate.uplift_base={:?}", self.uplift_base).unwrap();
        writeln!(w, "climate.uplift_per_slope={:?}", self.uplift_per_slope).unwrap();
        writeln!(w, "climate.uplift_slope_max={:?}", self.uplift_slope_max).unwrap();
        writeln!(
            w,
            "climate.advection_wind_min={:?}",
            self.advection_wind_min
        )
        .unwrap();
        writeln!(
            w,
            "climate.local_fraction_equator={:?}",
            self.local_fraction_equator
        )
        .unwrap();
        writeln!(
            w,
            "climate.local_fraction_width_deg={:?}",
            self.local_fraction_width_deg
        )
        .unwrap();
        writeln!(w, "climate.belt_edges_c={:?}", self.belt_edges_c).unwrap();
        writeln!(w, "climate.province_edges_r={:?}", self.province_edges_r).unwrap();
        writeln!(w, "climate.winter_boreal_c={:?}", self.winter_boreal_c).unwrap();
        writeln!(
            w,
            "climate.winter_temperate_c={:?}",
            self.winter_temperate_c
        )
        .unwrap();
        writeln!(w, "climate.cold_desert_sand={:?}", self.cold_desert_sand).unwrap();
        writeln!(
            w,
            "climate.cold_desert_winter_c={:?}",
            self.cold_desert_winter_c
        )
        .unwrap();
        writeln!(
            w,
            "climate.jitter_lambda_blocks={:?}",
            self.jitter_lambda_blocks
        )
        .unwrap();
        writeln!(w, "climate.jitter_octaves={:?}", self.jitter_octaves).unwrap();
        writeln!(w, "climate.jitter_gain={:?}", self.jitter_gain).unwrap();
        writeln!(w, "climate.sst_bands_c={:?}", self.sst_bands_c).unwrap();
        writeln!(w, "climate.c_sea_e_km={:?}", self.c_sea_e_km).unwrap();
        writeln!(w, "climate.c_sea={:?}", self.c_sea).unwrap();
        writeln!(w, "climate.c_land_iota_l={:?}", self.c_land_iota_l).unwrap();
        writeln!(w, "climate.c_land={:?}", self.c_land).unwrap();
        writeln!(w, "climate.c_coast_e_km={:?}", self.c_coast_e_km).unwrap();
        writeln!(w, "climate.c_coast={:?}", self.c_coast).unwrap();
        writeln!(w, "climate.ocean_c_threshold={:?}", self.ocean_c_threshold).unwrap();
        writeln!(w, "climate.e_base={:?}", self.e_base).unwrap();
        writeln!(w, "climate.e_noise={:?}", self.e_noise).unwrap();
        writeln!(w, "climate.e_mnt={:?}", self.e_mnt).unwrap();
        writeln!(w, "climate.e_mnt_noise={:?}", self.e_mnt_noise).unwrap();
        writeln!(w, "climate.e_plateau={:?}", self.e_plateau).unwrap();
        writeln!(w, "climate.m_e_lo={:?}", self.m_e_lo).unwrap();
        writeln!(w, "climate.m_e_hi={:?}", self.m_e_hi).unwrap();
        writeln!(w, "climate.w_plain_base={:?}", self.w_plain_base).unwrap();
        writeln!(w, "climate.w_plain_noise={:?}", self.w_plain_noise).unwrap();
        writeln!(w, "climate.w_mnt_base={:?}", self.w_mnt_base).unwrap();
        writeln!(w, "climate.w_mnt_noise={:?}", self.w_mnt_noise).unwrap();
        writeln!(w, "climate.w_plateau={:?}", self.w_plateau).unwrap();
        writeln!(w, "climate.w_max={:?}", self.w_max).unwrap();
        writeln!(w, "climate.w_sign_scale={:?}", self.w_sign_scale).unwrap();
        writeln!(w, "climate.t_centers={:?}", self.t_centers).unwrap();
        writeln!(w, "climate.h_centers={:?}", self.h_centers).unwrap();
        writeln!(w, "climate.h4_base={:?}", self.h4_base).unwrap();
        writeln!(w, "climate.h4_gain={:?}", self.h4_gain).unwrap();
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub version: GenVersion,
    pub mode: Mode,
    pub block: BlockParams,
    pub plates: PlatesParams,
    pub crust: CrustParams,
    pub poles: PolesParams,
    pub wind: WindParams,
    pub precip: PrecipParams,
    pub rivers: RiversParams,
    pub drainage: DrainageParams,
    pub channels: ChannelParams,
    pub lakes: LakeParams,
    pub hydrology: HydrologyParams,
    pub lava: LavaParams,
    pub spawn: SpawnParams,
    pub lattice: LatticeParams,
    pub climate: ClimateParams,
    /// Инфраструктурная подпись конфигурации для кэшей кандидатов (Д-027).
    /// Намеренно НЕ входит в `canonical_text()` / `params_hash()` (9.5):
    /// влияет только на скорость, не на мир.
    pub(crate) config_key: u64,
}

impl Params {
    /// Стартовые значения v0 из приложения D. Макро-поля не зависят от режима
    /// (8.2); `block` берётся из `Mode::block_params`.
    pub fn defaults(version: GenVersion, mode: Mode) -> Self {
        let block = mode.block_params();
        let mut p = Self {
            version,
            mode,
            block,
            plates: PlatesParams {
                cell_g_l: 16.0,
                inset_fraction: 0.08,
                // Д-021: больше плит с материком, разброс размеров вместо
                // россыпи равновеликих «Грендландий».
                empty_probability: 0.22,
                double_probability: 0.35,
                min_seed_distance_fraction: 0.35,
                seed_attempts: 4,
                center_offset_fraction: 0.25,
                // Д-021: вместо равновеликих «Грендландий» — степенной разброс:
                // единицы крупных, несколько средних, россыпь мелких.
                continent_probability: 0.45,
                continent_radius_min_l: 2.5,
                continent_radius_max_l: 17.5,
                continent_radius_skew: 2.2,
                aspect_min: 1.0,
                aspect_max: 2.0,
                speed_min: 0.2,
                speed_max: 1.0,
                omega_max: 0.03,
                warp_lambda0_l: 24.0,
                // Д-011: 1.6 L давало почти прямые многоугольники («сетку»).
                // 4.0 L сохраняет якобиан > 0.2 на контрольной сетке.
                warp_amplitude_l: 4.0,
                warp_octaves: 4,
                warp_gain: 0.45,
                query_radius_cells: 2,
                triple_weight_l: 0.4,
                boundary_conv_threshold: 0.1,
                boundary_shear_threshold: 0.15,
                boundary_width: 0.5,
            },
            crust: CrustParams {
                shelf_depth_km: -0.2,
                abyss_depth_km: -4.5,
                slope_start_l: 0.5,
                slope_end_l: 1.5,
                continent_height_km: 0.6,
                h_col_km: 4.5,
                w_col_l: 0.6,
                h_trench_km: 3.5,
                d_trench_l: 0.15,
                w_trench_l: 0.24,
                h_arc_km: 4.0,
                d_arc_l: 0.9,
                w_arc_l: 0.45,
                h_island_km: 4.3,
                d_island_l: 0.55,
                w_island_l: 0.28,
                h_ridge_km: 2.2,
                w_ridge_l: 1.8,
                h_axial_km: 0.4,
                w_axial_l: 0.12,
                h_rift_km: 0.9,
                w_rift_l: 0.35,
                h_shoulder_km: 0.7,
                ocean_kappa_l: -6.0,
                // Д-021: вытянутые эллипсы (r до 17,5 L, aspect до 2) на краю
                // окна 7×7 давали вклад выше уровня океана и прямой шов;
                // окно 9×9 возвращает тест Д-012 (< 1e-3).
                continent_query_radius_cells: 4,
                // Д-011: береговая линия должна быть изрезанной, а не повторять
                // границы ячеек Вороного.
                // Д-021: прибрежная рябь — четыре октавы от 6 L до 0,75 L.
                // Берег перестаёт быть гладким эллипсом. Частоты крупнее
                // coarse-решётки (0,25 L): иначе эталон и быстрый путь видят
                // разный климат и зоны расходятся (тест 14.2, Д-021).
                kappa_noise_amplitude_l: 3.0,
                kappa_noise_lambda_l: 6.0,
                kappa_noise_octaves: 4,
                kappa_noise_gain: 0.5,
                blend_width_l: 1.0,
                boundary_mix_factor: 0.5,
                shelf_start_l: 0.5,
                shelf_end_l: 1.5,
                continent_scale_l: 1.5,
                slope_drop_km: 4.3,
                plateau_factor: 0.4,
                island_noise_lambda_l: 1.2,
                island_noise_base: 0.55,
                island_noise_amp: 0.45,
                rift_shoulder_offset: 1.6,
                u6_offset_l: 0.8,
                u6_width_l: 0.5,
                u6_drop_km: 0.2,
                u7_amplitude_km: 0.35,
                u7_width_l: 0.24,
                u7_noise_lambda_l: 2.0,
                mountain_edge_l: 0.3,
                mountain_scale_km: 3.0,
                mountain_transform_factor: 0.25,
                detail_land_km: 0.25,
                detail_ocean_km: 0.3,
                detail_mountain_km: 0.6,
                detail_land_lambda_l: 3.0,
                detail_ocean_lambda_l: 4.0,
                detail_mountain_lambda_l: 0.4,
                detail_octaves: 3,
                detail_gain: 0.5,
                iota_height_factor: 0.5,
                iota_max_l: 1.0,
                hotspot_inset: 0.1,
                hotspot_chain_count: 4,
                hotspot_chain_step_l: 0.9,
                hotspot_chain_decay: 0.17,
                hotspot_radius_growth: 0.1,
                // Д-021: короткие цепочки в мелкой сетке = отдельные острова.
                hotspot_continental_factor: 0.85,
                hotspot_consider_radius_l: 2.5,
                hotspot_cell_l: 20.0,
                hotspot_probability: 0.45,
                h_hotspot_km: 4.6,
                r_hotspot_l: 0.7,
            },
            poles: PolesParams {
                // Д-021: полюса в два раза дальше (128 L вместо 64 L):
                // на кадре 32,8 км больше не стоят четыре шапки подряд.
                cell_g_p_l: 128.0,
                inset_fraction: 0.1,
                // Рваные, изогнутые границы холода вместо концентрических кругов.
                // λ₀ = 40 L и 3 октавы: даже мелкая октава (10 L) много крупнее
                // coarse-узла, поэтому быстрый путь видит тот же изгиб.
                warp_lambda0_l: 40.0,
                warp_amplitude_l: 6.0,
                warp_octaves: 3,
                warp_gain: 0.5,
                query_radius_cells: 2,
                // Д-021: 6° дрожи ломали луч влаги (32 шага × 0,25 L):
                // ветер в coarse-узле и в точке уходил разными путями, p_mm
                // расходился в разы и зоны не совпадали. 3° + изгиб warp 6 L
                // дают рваные кольца без разлада быстрого пути.
                phi_noise_degrees: 3.0,
                phi_noise_lambda_l: 12.0,
                phi_noise_octaves: 3,
                phi_noise_gain: 0.5,
            },
            wind: WindParams {
                strength_phi_deg: [0.0, 5.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0],
                strength_s: [0.25, 0.30, 1.00, 0.30, 1.00, 0.40, 0.60, 0.30],
            },
            precip: PrecipParams {
                ray_steps: 32,
                ray_step_l: 0.25,
                ocean_length_l: 1.0,
                // Д-023: над сушей влага жила 6 L, а материки имеют радиус
                // 9–17,5 L и луч влаги всего 8 L. Внутри континента μ падала
                // до 0,15, осадки до 448 мм при PET 962 мм — отсюда 38 %
                // пустынь и 9 % лесов на 16 сидах. Характерная длина удвоена.
                land_length_l: 32.0,
                orographic_height_km: 1.5,
                itcz_precip_mm: 1900.0,
                itcz_width_deg: 11.0,
                front_precip_mm: 900.0,
                front_center_deg: 50.0,
                front_width_deg: 13.0,
                local_fraction_base: 0.15,
                advection_mm: 1500.0,
            },
            rivers: RiversParams {
                source_cell_l: 2.0,
                source_probability: 0.6,
                source_min_mountain: 0.35,
                source_min_height_km: 0.8,
                source_min_precip_mm: 500.0,
                source_window_cells: 2,
                trace_step_l: 0.05,
                trace_max_steps: 60,
                trace_min_length_l: 1.0,
                trace_gradient_min: 0.02,
                trace_flat_steps: 5,
                meander_turn_rad: 0.6,
                gradient_step_l: 0.05,
                foothill_sigma_factor: -1.6,
                half_width_growth: 0.6,
                half_width_length_l: 3.0,
            },
            drainage: DrainageParams {
                step_l: 0.25,
                jitter_fraction: 0.35,
                fill_epsilon_km: 1e-4,
                // Стартовый отсев «луж» глубже 2 м; подбор оставлен T151.
                depression_min_depth_km: 0.002,
                outlet_min_weight: 0.5,
                anchor_search_radius_cells: 2,
                climate_step_l: 1.0,
                et_factor: 1.0,
            },
            channels: ChannelParams {
                // Стартовый отсев русел; плотность подбирается T151 по 10.1.
                q_min_mm_l2: 25.0,
                core_min_blocks: match mode {
                    Mode::Gameplay => 4.0,
                    Mode::Scientific => 6.0,
                },
                core_max_blocks: match mode {
                    Mode::Gameplay => 28.0,
                    Mode::Scientific => 60.0,
                },
                depth0_blocks: match mode {
                    Mode::Gameplay => 1.5,
                    Mode::Scientific => 2.0,
                },
                meander_plain: 1.5,
                meander_mountain: 0.3,
                valley_plain: 5.0,
                valley_mountain: 2.0,
                plain_slope_max: 0.02,
                // Стартовые числа §5.4; подбор плотности/болот — T151.
                delta_q_min_mm_l2: 500.0,
                delta_slope_max: 0.005,
                delta_length_min_l: 0.3,
                delta_length_max_l: 0.8,
            },
            lakes: LakeParams {
                inland_sea_area_l2: 1.0,
                swamp_max_depth_km: 0.01,
                swamp_min_p_pet: 1.2,
                frozen_t_min_c: 0.0,
                shore_fraction: 0.5,
            },
            hydrology: HydrologyParams {
                bank_fraction: 0.5,
                field_fade_l: 0.48,
                floodplain_e: 0.3,
                swamp_e: 0.65,
                dry_basin_e: 0.3,
                estuary_c: -0.20,
                estuary_elevation_km: 0.15,
                inland_sea_c: -0.25,
                r1_max_level_km: 0.3,
            },
            lava: LavaParams {
                width_blocks: block.lava_width_blocks,
                background: block.lava_background,
                base_factor: 0.4,
                boundary_factor: 0.6,
            },
            spawn: SpawnParams {
                // Д-021: полюса раздвинулись, поэтому ищем спавн в 200 L
                // и берём больше проб, чтобы кольцо 25…55° гарантированно нашлось.
                candidates: 384,
                radius_l: 200.0,
                fallback_grid: 48,
                min_e_km: 0.1,
                max_e_km: 1.5,
                min_kappa_l: 1.0,
                max_mountain: 0.2,
                min_phi_deg: 25.0,
                max_phi_deg: 55.0,
                min_ratio: 0.5,
                max_ratio: 2.0,
                fixed_mu: 0.4,
                fixed_wind: 0.6,
                search_radius_blocks: 2048.0,
            },
            climate: ClimateParams {
                t_equator_c: 28.0,
                t_pole_drop_c: 52.0,
                t_lat_exponent: 1.6,
                lapse_c_per_km: 6.5,
                cont_tropical_c: 2.0,
                cont_polar_c: 6.0,
                amp_base_c: 1.0,
                amp_lat_c: 24.0,
                amp_sea_factor: 0.25,
                amp_land_factor: 0.75,
                sst_min_c: -1.8,
                bio_min_c: 0.0,
                bio_max_c: 30.0,
                coldest_cos: 0.965_925_8,
                trade_n: -0.5,
                trade_e: -0.87,
                west_n: 0.35,
                west_e: 0.94,
                polar_n: -0.5,
                polar_e: -0.87,
                belt_trade_deg: [25.0, 35.0],
                belt_west_deg: [25.0, 35.0],
                // Д-021: полярный пояс уже, шапки меньше.
                belt_polar_deg: [68.0, 78.0],
                wind_fallback_dir: [-0.87, 0.5],
                w_sat_base: 0.50,
                w_sat_per_c: 0.048,
                w_sat_min: 0.3,
                w_sat_max: 2.1,
                // Д-023: если дальний конец 8-L луча тоже суша (а при
                // континентальности 0,84 это почти всегда), стартовая влага
                // не должна обнуляться вдвое.
                w_land_start: 0.85,
                cont_decay_l: 3.0,
                c_iso_end_l: 5.0,
                c_ray_wind_lo: 0.1,
                c_ray_wind_hi: 0.4,
                pet_per_bio_c: 58.93,
                p_min_mm: 10.0,
                slope_probe_l: 0.25,
                uplift_base: 0.6,
                uplift_per_slope: 0.5,
                uplift_slope_max: 3.0,
                advection_wind_min: 0.6,
                local_fraction_equator: 0.55,
                local_fraction_width_deg: 15.0,
                belt_edges_c: [1.5, 3.0, 6.0, 12.0, 18.0, 24.0],
                province_edges_r: [0.5, 1.0, 2.0, 4.0, 8.0],
                winter_boreal_c: -12.0,
                winter_temperate_c: -8.0,
                cold_desert_sand: true,
                cold_desert_winter_c: 0.0,
                jitter_lambda_blocks: 240.0,
                jitter_octaves: 3,
                jitter_gain: 0.5,
                sst_bands_c: [0.0, 10.0, 18.0, 24.0],
                c_sea_e_km: [-8.0, -4.5, -3.5, -2.5, -1.5, -0.2, 0.0],
                c_sea: [-0.98, -0.70, -0.52, -0.45, -0.40, -0.30, -0.17],
                c_land_iota_l: [0.0, 0.05, 0.5, 2.0, 5.0, 10.0, 20.0],
                c_land: [-0.17, -0.11, 0.03, 0.30, 0.60, 0.85, 1.00],
                c_coast_e_km: 0.60,
                c_coast: -0.17,
                ocean_c_threshold: -0.19,
                e_base: 0.25,
                e_noise: 0.15,
                e_mnt: -0.75,
                e_mnt_noise: 0.25,
                e_plateau: -0.30,
                m_e_lo: 0.05,
                m_e_hi: 1.2,
                w_plain_base: 0.18,
                w_plain_noise: 0.10,
                w_mnt_base: 0.30,
                w_mnt_noise: 0.40,
                w_plateau: 0.33,
                w_max: 0.75,
                w_sign_scale: 3.0,
                w_sign_epsilon: 0.0,
                t_centers: [-0.70, -0.30, 0.00, 0.37, 0.77],
                h_centers: [-0.65, -0.22, 0.00, 0.20, 0.45],
                h4_base: 0.45,
                h4_gain: 0.40,
            },
            lattice: LatticeParams {
                fine_divisor: 16.0,
                coarse_divisor: 4.0,
                tile_nodes: 16,
                halo_nodes: 2,
                cache_shards: 64,
            },
            config_key: 0,
        };
        // T103 / Д-030: отдельный набор Э3.5. Ни одно число v0 не меняется.
        // *_noise у fBm и jitter теперь означает σ, а не амплитуду сырого fBm.
        // w_mnt_noise остаётся амплитудой положительного ridged2, не fBm.
        if version.is_water_relief_dev() {
            p.block.lambda_e_blocks *= 2.0;
            p.block.lambda_s_blocks = match mode {
                Mode::Gameplay => 2400.0,
                Mode::Scientific => 8000.0,
            };
            // Старт плана 12/24 дал max |ΔW|=0.03478 на seams-скане при
            // пороге 0.02 (CI 42493770230). Порог не ослабляем: расширяем
            // сглаживание знака, а не русло. Полоса |W|<0.05 остаётся узкой.
            p.block.w_sign_half_width_blocks = match mode {
                Mode::Gameplay => 48.0,
                Mode::Scientific => 96.0,
            };
            p.block.jitter_t_c = match mode {
                Mode::Gameplay => 1.0,
                Mode::Scientific => 0.5,
            };
            p.block.jitter_r_log2 = p.block.jitter_t_c * 0.25;
            let c = &mut p.climate;
            c.c_coast_e_km = 0.20;
            c.e_base = 0.15;
            c.e_noise = 0.20;
            c.w_plain_base = 0.30;
            c.w_plain_noise = 0.10;
            c.w_sign_epsilon = 1e-3;
        }
        p.config_key = p.params_hash();
        p
    }

    /// Каноническая текстовая запись: только поля и значения, без пробелов в
    /// именах, в фиксированном порядке. Хэш от неё стабилен между платформами.
    pub fn canonical_text(&self) -> String {
        let mut out = String::new();
        let w = &mut out;
        writeln!(w, "version={v}", v = self.version.as_str()).unwrap();
        writeln!(w, "mode={v}", v = self.mode.as_str()).unwrap();
        let b = &self.block;
        writeln!(w, "block.l_blocks={b_l}", b_l = b.l_blocks).unwrap();
        writeln!(w, "block.delta_fine_blocks={v}", v = b.delta_fine_blocks).unwrap();
        writeln!(
            w,
            "block.delta_coarse_blocks={v}",
            v = b.delta_coarse_blocks
        )
        .unwrap();
        writeln!(w, "block.lambda_e_blocks={v}", v = b.lambda_e_blocks).unwrap();
        writeln!(w, "block.lambda_w_blocks={v}", v = b.lambda_w_blocks).unwrap();
        writeln!(w, "block.lambda_s_blocks={v}", v = b.lambda_s_blocks).unwrap();
        writeln!(
            w,
            "block.river_half_width_blocks={v}",
            v = b.river_half_width_blocks
        )
        .unwrap();
        writeln!(
            w,
            "block.meander_amplitude_blocks={v}",
            v = b.meander_amplitude_blocks
        )
        .unwrap();
        writeln!(
            w,
            "block.meander_lambda_blocks={v}",
            v = b.meander_lambda_blocks
        )
        .unwrap();
        writeln!(w, "block.jitter_t_c={v}", v = b.jitter_t_c).unwrap();
        writeln!(w, "block.jitter_r_log2={v}", v = b.jitter_r_log2).unwrap();
        writeln!(w, "block.lava_width_blocks={v}", v = b.lava_width_blocks).unwrap();
        writeln!(w, "block.lava_background={v}", v = b.lava_background).unwrap();
        writeln!(w, "block.swamps_enabled={v}", v = b.swamps_enabled).unwrap();
        writeln!(
            w,
            "block.mushroom_islands_enabled={v}",
            v = b.mushroom_islands_enabled
        )
        .unwrap();
        let p = &self.plates;
        writeln!(w, "plates.cell_g_l={v}", v = p.cell_g_l).unwrap();
        writeln!(w, "plates.inset_fraction={v}", v = p.inset_fraction).unwrap();
        writeln!(w, "plates.empty_probability={v}", v = p.empty_probability).unwrap();
        writeln!(w, "plates.double_probability={v}", v = p.double_probability).unwrap();
        writeln!(
            w,
            "plates.min_seed_distance_fraction={v}",
            v = p.min_seed_distance_fraction
        )
        .unwrap();
        writeln!(w, "plates.seed_attempts={v}", v = p.seed_attempts).unwrap();
        writeln!(
            w,
            "plates.center_offset_fraction={v}",
            v = p.center_offset_fraction
        )
        .unwrap();
        writeln!(
            w,
            "plates.continent_probability={v}",
            v = p.continent_probability
        )
        .unwrap();
        writeln!(
            w,
            "plates.continent_radius_min_l={v}",
            v = p.continent_radius_min_l
        )
        .unwrap();
        writeln!(
            w,
            "plates.continent_radius_max_l={v}",
            v = p.continent_radius_max_l
        )
        .unwrap();
        writeln!(
            w,
            "plates.continent_radius_skew={v}",
            v = p.continent_radius_skew
        )
        .unwrap();
        writeln!(w, "plates.aspect_min={v}", v = p.aspect_min).unwrap();
        writeln!(w, "plates.aspect_max={v}", v = p.aspect_max).unwrap();
        writeln!(w, "plates.speed_min={v}", v = p.speed_min).unwrap();
        writeln!(w, "plates.speed_max={v}", v = p.speed_max).unwrap();
        writeln!(w, "plates.omega_max={v}", v = p.omega_max).unwrap();
        writeln!(w, "plates.warp_lambda0_l={v}", v = p.warp_lambda0_l).unwrap();
        writeln!(w, "plates.warp_amplitude_l={v}", v = p.warp_amplitude_l).unwrap();
        writeln!(w, "plates.warp_octaves={v}", v = p.warp_octaves).unwrap();
        writeln!(w, "plates.warp_gain={v}", v = p.warp_gain).unwrap();
        writeln!(w, "plates.query_radius_cells={v}", v = p.query_radius_cells).unwrap();
        writeln!(w, "plates.triple_weight_l={v}", v = p.triple_weight_l).unwrap();
        writeln!(
            w,
            "plates.boundary_conv_threshold={v}",
            v = p.boundary_conv_threshold
        )
        .unwrap();
        writeln!(
            w,
            "plates.boundary_shear_threshold={v}",
            v = p.boundary_shear_threshold
        )
        .unwrap();
        writeln!(w, "plates.boundary_width={v}", v = p.boundary_width).unwrap();
        let c = &self.crust;
        writeln!(w, "crust.h_col_km={v}", v = c.h_col_km).unwrap();
        writeln!(w, "crust.w_col_l={v}", v = c.w_col_l).unwrap();
        writeln!(w, "crust.h_trench_km={v}", v = c.h_trench_km).unwrap();
        writeln!(w, "crust.h_ridge_km={v}", v = c.h_ridge_km).unwrap();
        writeln!(w, "crust.h_rift_km={v}", v = c.h_rift_km).unwrap();
        writeln!(w, "crust.h_hotspot_km={v}", v = c.h_hotspot_km).unwrap();
        writeln!(
            w,
            "crust.hotspot_probability={v}",
            v = c.hotspot_probability
        )
        .unwrap();
        w.push_str(&self.crust.extra_text());
        w.push_str(&self.climate.extra_text());
        let pol = &self.poles;
        writeln!(w, "poles.cell_g_p_l={v}", v = pol.cell_g_p_l).unwrap();
        writeln!(w, "poles.warp_lambda0_l={v}", v = pol.warp_lambda0_l).unwrap();
        writeln!(w, "poles.warp_amplitude_l={v}", v = pol.warp_amplitude_l).unwrap();
        writeln!(
            w,
            "poles.query_radius_cells={v}",
            v = pol.query_radius_cells
        )
        .unwrap();
        writeln!(w, "poles.phi_noise_degrees={v}", v = pol.phi_noise_degrees).unwrap();
        writeln!(
            w,
            "poles.phi_noise_lambda_l={v}",
            v = pol.phi_noise_lambda_l
        )
        .unwrap();
        writeln!(w, "poles.phi_noise_octaves={v}", v = pol.phi_noise_octaves).unwrap();
        writeln!(w, "poles.phi_noise_gain={v}", v = pol.phi_noise_gain).unwrap();
        for (i, (phi, s)) in self
            .wind
            .strength_phi_deg
            .iter()
            .zip(self.wind.strength_s.iter())
            .enumerate()
        {
            writeln!(w, "wind.knot{i}.phi_deg={phi}").unwrap();
            writeln!(w, "wind.knot{i}.s={s}").unwrap();
        }
        let pr = &self.precip;
        writeln!(w, "precip.ray_steps={v}", v = pr.ray_steps).unwrap();
        writeln!(w, "precip.ray_step_l={v}", v = pr.ray_step_l).unwrap();
        writeln!(w, "precip.ocean_length_l={v}", v = pr.ocean_length_l).unwrap();
        writeln!(w, "precip.land_length_l={v}", v = pr.land_length_l).unwrap();
        writeln!(
            w,
            "precip.orographic_height_km={v}",
            v = pr.orographic_height_km
        )
        .unwrap();
        writeln!(w, "precip.itcz_precip_mm={v}", v = pr.itcz_precip_mm).unwrap();
        writeln!(w, "precip.advection_mm={v}", v = pr.advection_mm).unwrap();
        let r = &self.rivers;
        for (name, value) in [
            ("rivers.source_cell_l", r.source_cell_l),
            ("rivers.source_probability", r.source_probability),
            ("rivers.source_min_mountain", r.source_min_mountain),
            ("rivers.source_min_height_km", r.source_min_height_km),
            ("rivers.source_min_precip_mm", r.source_min_precip_mm),
            ("rivers.trace_step_l", r.trace_step_l),
            ("rivers.trace_min_length_l", r.trace_min_length_l),
            ("rivers.trace_gradient_min", r.trace_gradient_min),
            ("rivers.meander_turn_rad", r.meander_turn_rad),
            ("rivers.gradient_step_l", r.gradient_step_l),
            ("rivers.foothill_sigma_factor", r.foothill_sigma_factor),
            ("rivers.half_width_growth", r.half_width_growth),
            ("rivers.half_width_length_l", r.half_width_length_l),
        ] {
            writeln!(w, "{name}={value:?}").unwrap();
        }
        writeln!(w, "rivers.trace_max_steps={v}", v = r.trace_max_steps).unwrap();
        writeln!(w, "rivers.trace_flat_steps={v}", v = r.trace_flat_steps).unwrap();
        writeln!(
            w,
            "rivers.source_window_cells={v}",
            v = r.source_window_cells
        )
        .unwrap();
        writeln!(w, "lava.width_blocks={v}", v = self.lava.width_blocks).unwrap();
        writeln!(w, "lava.background={v}", v = self.lava.background).unwrap();
        writeln!(w, "lava.base_factor={:?}", self.lava.base_factor).unwrap();
        writeln!(w, "lava.boundary_factor={:?}", self.lava.boundary_factor).unwrap();
        writeln!(w, "spawn.candidates={v}", v = self.spawn.candidates).unwrap();
        writeln!(w, "spawn.radius_l={v}", v = self.spawn.radius_l).unwrap();
        for (name, value) in [
            ("spawn.min_e_km", self.spawn.min_e_km),
            ("spawn.max_e_km", self.spawn.max_e_km),
            ("spawn.min_kappa_l", self.spawn.min_kappa_l),
            ("spawn.max_mountain", self.spawn.max_mountain),
            ("spawn.min_phi_deg", self.spawn.min_phi_deg),
            ("spawn.max_phi_deg", self.spawn.max_phi_deg),
            ("spawn.min_ratio", self.spawn.min_ratio),
            ("spawn.max_ratio", self.spawn.max_ratio),
            ("spawn.fixed_mu", self.spawn.fixed_mu),
            ("spawn.fixed_wind", self.spawn.fixed_wind),
            (
                "spawn.search_radius_blocks",
                self.spawn.search_radius_blocks,
            ),
        ] {
            writeln!(w, "{name}={value:?}").unwrap();
        }
        writeln!(w, "spawn.fallback_grid={v}", v = self.spawn.fallback_grid).unwrap();
        let l = &self.lattice;
        writeln!(w, "lattice.fine_divisor={v}", v = l.fine_divisor).unwrap();
        writeln!(w, "lattice.coarse_divisor={v}", v = l.coarse_divisor).unwrap();
        writeln!(w, "lattice.tile_nodes={v}", v = l.tile_nodes).unwrap();
        writeln!(w, "lattice.halo_nodes={v}", v = l.halo_nodes).unwrap();
        writeln!(w, "lattice.cache_shards={v}", v = l.cache_shards).unwrap();
        // Новые ключи дописываются только для новой версии. Побайтовая запись
        // v0 остаётся прежней, включая порядок строк иDebug f64.
        if self.version.is_water_relief_dev() {
            writeln!(
                w,
                "block.w_sign_half_width_blocks={v}",
                v = b.w_sign_half_width_blocks
            )
            .unwrap();
            writeln!(
                w,
                "climate.w_sign_epsilon={v}",
                v = self.climate.w_sign_epsilon
            )
            .unwrap();
            writeln!(w, "noise.amplitude_units=sigma").unwrap();
            let d = &self.drainage;
            for (name, value) in [
                ("drainage.step_l", d.step_l),
                ("drainage.jitter_fraction", d.jitter_fraction),
                ("drainage.fill_epsilon_km", d.fill_epsilon_km),
                (
                    "drainage.depression_min_depth_km",
                    d.depression_min_depth_km,
                ),
                ("drainage.outlet_min_weight", d.outlet_min_weight),
                ("drainage.climate_step_l", d.climate_step_l),
                ("drainage.et_factor", d.et_factor),
            ] {
                writeln!(w, "{name}={value:?}").unwrap();
            }
            writeln!(
                w,
                "drainage.anchor_search_radius_cells={v}",
                v = d.anchor_search_radius_cells
            )
            .unwrap();
            let ch = &self.channels;
            for (name, value) in [
                ("channels.q_min_mm_l2", ch.q_min_mm_l2),
                ("channels.core_min_blocks", ch.core_min_blocks),
                ("channels.core_max_blocks", ch.core_max_blocks),
                ("channels.depth0_blocks", ch.depth0_blocks),
                ("channels.meander_plain", ch.meander_plain),
                ("channels.meander_mountain", ch.meander_mountain),
                ("channels.valley_plain", ch.valley_plain),
                ("channels.valley_mountain", ch.valley_mountain),
                ("channels.plain_slope_max", ch.plain_slope_max),
                ("channels.delta_q_min_mm_l2", ch.delta_q_min_mm_l2),
                ("channels.delta_slope_max", ch.delta_slope_max),
                ("channels.delta_length_min_l", ch.delta_length_min_l),
                ("channels.delta_length_max_l", ch.delta_length_max_l),
            ] {
                writeln!(w, "{name}={value:?}").unwrap();
            }
            let lk = &self.lakes;
            for (name, value) in [
                ("lakes.inland_sea_area_l2", lk.inland_sea_area_l2),
                ("lakes.swamp_max_depth_km", lk.swamp_max_depth_km),
                ("lakes.swamp_min_p_pet", lk.swamp_min_p_pet),
                ("lakes.frozen_t_min_c", lk.frozen_t_min_c),
                ("lakes.shore_fraction", lk.shore_fraction),
            ] {
                writeln!(w, "{name}={value:?}").unwrap();
            }
            let hy = &self.hydrology;
            for (name, value) in [
                ("hydrology.bank_fraction", hy.bank_fraction),
                ("hydrology.field_fade_l", hy.field_fade_l),
                ("hydrology.floodplain_e", hy.floodplain_e),
                ("hydrology.swamp_e", hy.swamp_e),
                ("hydrology.dry_basin_e", hy.dry_basin_e),
                ("hydrology.estuary_c", hy.estuary_c),
                ("hydrology.estuary_elevation_km", hy.estuary_elevation_km),
                ("hydrology.inland_sea_c", hy.inland_sea_c),
                ("hydrology.r1_max_level_km", hy.r1_max_level_km),
            ] {
                writeln!(w, "{name}={value:?}").unwrap();
            }
        }
        out
    }

    pub fn params_hash(&self) -> u64 {
        fnv1a64(&self.canonical_text())
    }
}

#[cfg(test)]
mod tests {
    use super::Params;
    use crate::mode::Mode;
    use crate::version::GenVersion;

    #[test]
    fn macro_params_do_not_depend_on_mode() {
        let sci = Params::defaults(GenVersion::V0, Mode::Scientific);
        let game = Params::defaults(GenVersion::V0, Mode::Gameplay);
        assert_eq!(sci.plates, game.plates);
        assert_eq!(sci.crust, game.crust);
        assert_eq!(sci.poles, game.poles);
        assert_eq!(sci.precip, game.precip);
        assert_eq!(sci.climate, game.climate);
        assert_ne!(sci.block, game.block);
    }

    #[test]
    fn v0_is_frozen_and_v1_dev_has_its_own_parameters() {
        for (mode, hash) in [
            (Mode::Scientific, 0x5cdbe1d36ef8d16a),
            (Mode::Gameplay, 0xb48e8037c4e85977),
        ] {
            let old = Params::defaults(GenVersion::V0, mode);
            assert_eq!(old.params_hash(), hash);
            assert!(!old.canonical_text().contains("noise.amplitude_units"));
            let dev = Params::defaults(GenVersion::V1Dev, mode);
            assert_ne!(old.params_hash(), dev.params_hash());
            assert_eq!(dev.climate.c_coast_e_km, 0.20);
            assert_eq!(dev.climate.e_base, 0.15);
            assert_eq!(dev.climate.e_noise, 0.20);
            assert_eq!(dev.climate.w_plain_base, 0.30);
            assert_eq!(dev.block.lambda_e_blocks, 2.0 * old.block.lambda_e_blocks);
            assert!(dev.block.w_sign_half_width_blocks > 0.0);
            assert_eq!(dev.plates, old.plates);
            assert_eq!(dev.crust, old.crust);
            assert_eq!(dev.poles, old.poles);
        }
    }

    #[test]
    fn params_hash_is_stable_and_sensitive() {
        let a = Params::defaults(GenVersion::V0, Mode::Scientific);
        let b = Params::defaults(GenVersion::V0, Mode::Scientific);
        let c = Params::defaults(GenVersion::V0, Mode::Gameplay);
        assert_eq!(a.params_hash(), b.params_hash());
        assert_ne!(a.params_hash(), c.params_hash());
        assert!(a.canonical_text().contains("plates.cell_g_l=16"));
    }
}
