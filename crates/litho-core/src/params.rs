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
    pub trace_step_l: f64,
    pub trace_max_steps: usize,
    pub trace_min_length_l: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LavaParams {
    pub width_blocks: f64,
    pub background: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpawnParams {
    pub candidates: usize,
    pub radius_l: f64,
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
    pub lava: LavaParams,
    pub spawn: SpawnParams,
    pub lattice: LatticeParams,
}

impl Params {
    /// Стартовые значения v0 из приложения D. Макро-поля не зависят от режима
    /// (8.2); `block` берётся из `Mode::block_params`.
    pub fn defaults(version: GenVersion, mode: Mode) -> Self {
        let block = mode.block_params();
        Self {
            version,
            mode,
            block,
            plates: PlatesParams {
                cell_g_l: 16.0,
                inset_fraction: 0.08,
                empty_probability: 0.35,
                double_probability: 0.25,
                min_seed_distance_fraction: 0.35,
                seed_attempts: 4,
                center_offset_fraction: 0.25,
                continent_probability: 0.5,
                continent_radius_min_l: 4.0,
                continent_radius_max_l: 9.0,
                aspect_min: 1.0,
                aspect_max: 2.0,
                speed_min: 0.2,
                speed_max: 1.0,
                omega_max: 0.03,
                warp_lambda0_l: 24.0,
                warp_amplitude_l: 1.6,
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
                w_trench_l: 0.12,
                h_arc_km: 4.0,
                d_arc_l: 0.9,
                w_arc_l: 0.45,
                h_island_km: 4.2,
                d_island_l: 0.6,
                w_island_l: 0.18,
                h_ridge_km: 2.2,
                w_ridge_l: 1.8,
                h_axial_km: 0.4,
                w_axial_l: 0.06,
                h_rift_km: 0.9,
                w_rift_l: 0.25,
                h_shoulder_km: 0.7,
                hotspot_cell_l: 24.0,
                hotspot_probability: 0.3,
                h_hotspot_km: 4.8,
                r_hotspot_l: 0.5,
            },
            poles: PolesParams {
                cell_g_p_l: 64.0,
                inset_fraction: 0.1,
                warp_lambda0_l: 40.0,
                warp_amplitude_l: 4.0,
                warp_octaves: 3,
                warp_gain: 0.5,
                query_radius_cells: 2,
                phi_noise_degrees: 3.0,
            },
            wind: WindParams {
                strength_phi_deg: [0.0, 5.0, 15.0, 30.0, 45.0, 60.0, 75.0, 90.0],
                strength_s: [0.25, 0.30, 1.00, 0.30, 1.00, 0.40, 0.60, 0.30],
            },
            precip: PrecipParams {
                ray_steps: 32,
                ray_step_l: 0.25,
                ocean_length_l: 1.0,
                land_length_l: 6.0,
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
                trace_step_l: 0.05,
                trace_max_steps: 60,
                trace_min_length_l: 1.0,
            },
            lava: LavaParams {
                width_blocks: block.lava_width_blocks,
                background: block.lava_background,
            },
            spawn: SpawnParams {
                candidates: 256,
                radius_l: 100.0,
            },
            lattice: LatticeParams {
                fine_divisor: 16.0,
                coarse_divisor: 4.0,
                tile_nodes: 16,
                halo_nodes: 2,
                cache_shards: 64,
            },
        }
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
        writeln!(w, "rivers.source_cell_l={v}", v = r.source_cell_l).unwrap();
        writeln!(w, "rivers.trace_step_l={v}", v = r.trace_step_l).unwrap();
        writeln!(w, "rivers.trace_max_steps={v}", v = r.trace_max_steps).unwrap();
        writeln!(w, "lava.width_blocks={v}", v = self.lava.width_blocks).unwrap();
        writeln!(w, "lava.background={v}", v = self.lava.background).unwrap();
        writeln!(w, "spawn.candidates={v}", v = self.spawn.candidates).unwrap();
        writeln!(w, "spawn.radius_l={v}", v = self.spawn.radius_l).unwrap();
        let l = &self.lattice;
        writeln!(w, "lattice.fine_divisor={v}", v = l.fine_divisor).unwrap();
        writeln!(w, "lattice.coarse_divisor={v}", v = l.coarse_divisor).unwrap();
        writeln!(w, "lattice.tile_nodes={v}", v = l.tile_nodes).unwrap();
        writeln!(w, "lattice.halo_nodes={v}", v = l.halo_nodes).unwrap();
        writeln!(w, "lattice.cache_shards={v}", v = l.cache_shards).unwrap();
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
        assert_ne!(sci.block, game.block);
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
