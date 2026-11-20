//! W §6.6. Geometry is sampled in blocks; a site's cached result depends only
//! on the generator and the site, never on the requesting chunk or column.
use super::*;
use crate::math::rotate_unit;
use crate::seed::{CellRng, cell_hash, subseed};
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub(super) enum Family {
    Cirque,
    Glacial,
    Thermokarst,
    Sag,
}
const FAMILIES: [Family; 4] = [
    Family::Cirque,
    Family::Glacial,
    Family::Thermokarst,
    Family::Sag,
];
impl Family {
    fn dimensions(self, mode: Mode) -> (f64, f64, f64, f64) {
        use Family::*;
        // cell side, range of *major* radii, max depth (blocks)
        match (self, mode) {
            (Cirque, Mode::Gameplay) => (160., 8., 20., 6.),
            (Cirque, Mode::Scientific) => (320., 12., 32., 10.),
            (Glacial, Mode::Gameplay) => (96., 10., 28., 4.),
            (Glacial, Mode::Scientific) => (192., 16., 48., 6.),
            (Thermokarst, Mode::Gameplay) => (80., 12., 32., 2.),
            (Thermokarst, Mode::Scientific) => (160., 20., 60., 3.),
            (Sag, Mode::Gameplay) => (128., 8., 24., 3.),
            (Sag, Mode::Scientific) => (256., 16., 48., 5.),
        }
    }
    fn body(self) -> WaterBody {
        match self {
            Self::Cirque => WaterBody::Tarn,
            Self::Glacial => WaterBody::GlacialPond,
            Self::Thermokarst => WaterBody::Thermokarst,
            Self::Sag => WaterBody::SagPond,
        }
    }
}
#[derive(Clone, Copy, Debug)]
struct Site {
    center: Vec2,
    axis: Vec2,
    major: f64,
    minor: f64,
    depth: f64,
    family: Family,
    roll: f64,
}
impl Site {
    fn new(seed: u64, mode: Mode, family: Family, i: i64, j: i64) -> Self {
        let (side, lo, hi, depth) = family.dimensions(mode);
        let mut rng = CellRng::from_hash(cell_hash(
            subseed(seed, "water.point_lakes"),
            i,
            j,
            family as i64,
        ));
        let roll = rng.next_unit() * if mode == Mode::Scientific { 1.5 } else { 1. };
        let major = rng.next_range(lo, hi);
        let ratio = match family {
            Family::Cirque => 1.,
            Family::Glacial => rng.next_range(1., 2.),
            Family::Thermokarst => rng.next_range(1.5, 2.5),
            Family::Sag => 3.,
        };
        let minor = major / ratio;
        let pad = major + 4.;
        let center = Vec2::new(
            i as f64 * side + rng.next_range(pad, side - pad),
            j as f64 * side + rng.next_range(pad, side - pad),
        );
        let axis = rotate_unit(
            Vec2::new(1., 0.),
            rng.next_range(-std::f64::consts::PI, std::f64::consts::PI),
        );
        Self {
            center,
            axis,
            major,
            minor,
            depth,
            family,
            roll,
        }
    }
    // Bounded 3-lobed perturbation. Both contour probing and column inclusion
    // use this same factor. The perturbed curve stays inside the major radius.
    fn rim(&self, dir: Vec2) -> f64 {
        if self.family == Family::Glacial {
            0.94 + 0.06 * dir.z * (3. - 4. * dir.z * dir.z)
        } else {
            1.
        }
    }
    fn rho(&self, p: Vec2) -> f64 {
        let d = p.sub(self.center);
        let uv = Vec2::new(
            d.dot(self.axis) / self.major,
            d.dot(self.axis.perp()) / self.minor,
        );
        uv.len() / self.rim(uv.normalized())
    }
    fn ring_point(&self, dir: Vec2, pad: f64) -> Vec2 {
        let r = self.rim(dir);
        self.center
            .add(self.axis.scale(dir.x * (self.major * r + pad)))
            .add(self.axis.perp().scale(dir.z * (self.minor * r + pad)))
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Pond {
    site: Site,
    level: f64,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Hit {
    pub body: WaterBody,
    pub level: f64,
    pub depth: f64,
    pub rho: f64,
    pub edge: f64,
}
impl Hit {
    pub fn apply(
        self,
        mut h: crate::height::HeightSample,
        p: &Params,
    ) -> crate::height::HeightSample {
        let d = (self.edge - p.water.lake_calm_pad_blocks).max(0.);
        h.calm = h.calm.min(crate::math::smoothstep(
            p.water.calm_blocks[0],
            p.water.calm_blocks[1],
            d,
        ));
        if self.rho <= 1.0 {
            h.point_body = Some(self.body);
            h.water_y = self.level;
            h.bed_y = h
                .surface_y
                .min(self.level - 1. - self.depth * (1. - self.rho * self.rho));
            h.surface_y = h.bed_y;
            h.offset = (h.surface_y - crate::height::SURFACE_BASE) / crate::height::OFFSET_SCALE;
            h.wet_kind = hydrology::SurfaceKind::Lake;
            h.water_strength = 1.;
        } else if self.edge <= p.water.beach_blocks {
            h.point_shore = Some((self.body.point_material(), self.level as u16));
        }
        h
    }
}
type SiteSlot = Arc<OnceLock<Result<Option<Pond>, RiverNetError>>>;
pub(super) type SiteCache = ShardedCache<(Family, i64, i64), SiteSlot>;
pub(super) type TileCache = ShardedCache<(i64, i64), Result<Arc<Vec<Pond>>, RiverNetError>>;

impl Generator {
    fn pond_baseline(&self, p: Vec2) -> Result<FullSample, RiverNetError> {
        // Network-only surface, not water_column / cached_full: a site builder
        // must never request its own OnceLock, including through other sites.
        self.try_sample_with(p.x, p.z, SurfaceStage::Network)?
            .ok_or(RiverNetError::InvalidInput {
                cell: (p.x as i64, p.z as i64),
            })
    }
    /// Fast height-only S_rl. The network cut and bank use geometry only;
    /// `resolve` affects calm, not bed height. Humid ordinary beaches need the
    /// original precipitation decision, so fall back to the full network stage.
    fn pond_terrain(&self, p: Vec2) -> Result<(f64, bool, f64), RiverNetError> {
        let s = self.s0_node(p.x, p.z).ok_or(RiverNetError::InvalidInput {
            cell: (p.x as i64, p.z as i64),
        })?;
        let g = self.cached_hydrology_queries(p.x, p.z, true)?;
        let h = if g.lakes.shore_hit.is_some_and(|h| {
            !h.inside && h.distance_l * self.params.block.l_blocks <= self.params.water.beach_blocks
        }) {
            self.pond_baseline(p)?.world.height
        } else {
            crate::height::with_water_w(
                crate::height::HeightSample {
                    surface_y: s.macro_y,
                    ..Default::default()
                },
                s.micro_dy,
                &g,
                &self.params,
            )
        };
        Ok((h.surface_y, h.water_strength > 0., s.surface_y()))
    }
    fn build_pond(&self, mut site: Site) -> Result<Option<Pond>, RiverNetError> {
        use Family::*;
        let c = site.center;
        let max_prob = match site.family {
            Cirque => 0.35,
            Glacial => 0.5,
            Thermokarst => 0.6,
            Sag => 0.4,
        };
        if site.roll >= max_prob {
            return Ok(None);
        }
        let Some(weights) = self.morpho_weights(c.x, c.z) else {
            return Ok(None);
        };
        if match site.family {
            Glacial => {
                weights.values[0] + weights.values[1] < 0.5
                    || (weights.values[0] > weights.values[1] && site.roll >= 0.3)
            }
            Thermokarst => weights.values[0] < 0.5,
            _ => false,
        } {
            return Ok(None);
        }
        let Some(plate) = self.plate_sample(c.x, c.z) else {
            return Ok(None);
        };
        if site.family == Sag && plate.d_ab_l.abs() >= 0.1 {
            return Ok(None);
        }
        let probe = self
            .w_probe_known(
                c,
                &plate,
                &crust_from_sample(self.macro_point(c.x, c.z), &plate, self.seed, &self.params),
                false,
            )
            .ok_or(RiverNetError::InvalidInput {
                cell: (c.x as i64, c.z as i64),
            })?;
        if probe.macro_y < 65.
            || (site.family == Cirque
                && probe.macro_y
                    < if self.params.mode == Mode::Gameplay {
                        100.
                    } else {
                        140.
                    })
        {
            return Ok(None);
        }
        let center = self.pond_baseline(c)?;
        let climate = &center.climate;
        let terrain = center.world.height.surface_y;
        if terrain < 65. || center.world.height.water_strength > 0. {
            return Ok(None);
        }
        let l = self.params.block.l_blocks;
        let water = &center.world.hydrology;
        let near_channel = |hit: &crate::channel::ChannelHit| {
            let width = hit.core_l * l;
            hit.distance_l * l
                < 0.5 * width
                    + 2.
                    + self.params.water.floodplain_w[hit.valley_class.table_index()] * width
                    + 4.
        };
        if water
            .channels
            .nearest
            .iter()
            .chain(water.channels.extra.iter())
            .any(near_channel)
            || water
                .lakes
                .nearest
                .is_some_and(|h| h.inside || h.distance_l * l < 8.)
        {
            return Ok(None);
        }
        if site.family == Sag {
            if climate.surface.boundary_weights[6] < 0.5 {
                return Ok(None);
            }
            site.axis = plate.n_ab.perp().normalized();
        } else {
            if site.family == Cirque
                && weights.values[6] < 0.4
                && !(center.class.zone.belt <= 2 && probe.erosion_e < self.params.height.e_knots[2])
            {
                return Ok(None);
            }
            if site.family == Thermokarst {
                if climate.temperature.t_c > 1.5 {
                    return Ok(None);
                }
                if climate.wind.len() > 1e-9 {
                    site.axis = climate.wind.perp().normalized();
                }
            }
            let s = |dx, dz| {
                self.s0_node(c.x + dx, c.z + dz).map(|s| s.macro_y).ok_or(
                    RiverNetError::InvalidInput {
                        cell: (c.x as i64, c.z as i64),
                    },
                )
            };
            let slope = Vec2::new(s(2., 0.)? - s(-2., 0.)?, s(0., 2.)? - s(0., -2.)?).len() * 0.25;
            if match site.family {
                Cirque => !(0.15..=0.8).contains(&slope),
                Glacial => slope > 0.1,
                Thermokarst => slope > 0.05,
                Sag => false,
            } {
                return Ok(None);
            }
        }
        let mut low = f64::INFINITY;
        for pad in [0., 2., 4.] {
            let n = (std::f64::consts::PI * (site.major + pad) * 1.2).ceil() as usize;
            for i in 0..n {
                let dir = rotate_unit(
                    Vec2::new(1., 0.),
                    std::f64::consts::TAU * (i as f64 / n as f64 - 0.5),
                );
                let p = site.ring_point(dir, pad);
                let (height, wet, s0) = self.pond_terrain(p)?;
                low = low.min(height);
                if wet || low.floor() - 1. <= 64. || terrain - low.floor() + 1. > 8. {
                    return Ok(None);
                }
                if pad == 4. && s0 < 65. {
                    return Ok(None);
                }
            }
        }
        let level = low.floor() - 1.;
        // Check the *maximum*, not the mean of a few probes: a single hill in
        // an otherwise flat bowl is grounds for rejecting the whole feature.
        for z in (c.z - site.major).ceil() as i64..=(c.z + site.major).floor() as i64 {
            for x in (c.x - site.major).ceil() as i64..=(c.x + site.major).floor() as i64 {
                let p = Vec2::new(x as f64, z as f64);
                if site.rho(p) > 1. {
                    continue;
                }
                let (height, wet, _) = self.pond_terrain(p)?;
                if wet || height - level > 8. {
                    return Ok(None);
                }
            }
        }
        Ok(Some(Pond { site, level }))
    }
    fn cached_pond(&self, site: Site, i: i64, j: i64) -> Result<Option<Pond>, RiverNetError> {
        let slot = self
            .pond_sites
            .get_or_insert((site.family, i, j), || Arc::new(OnceLock::new()));
        *slot.get_or_init(|| self.build_pond(site))
    }
    /// Diagnostics: candidate centres inside a circle in **block** coordinates.
    /// Confirm the final body (including overlaps) through the same ABI column as the game.
    pub fn point_lake_examples(
        &self,
        body: WaterBody,
        center: Vec2,
        radius: f64,
        limit: usize,
    ) -> Result<Vec<(i64, i64, WaterColumn)>, RiverNetError> {
        let mut out = Vec::new();
        let Some(family) = FAMILIES.into_iter().find(|f| f.body() == body) else {
            return Ok(out);
        };
        if !self.params.version.is_water_update()
            || !self.params.water.point_lakes
            || !radius.is_finite()
            || radius < 0.
            || limit == 0
        {
            return Ok(out);
        }
        let (side, _, _, _) = family.dimensions(self.params.mode);
        for i in ((center.x - radius) / side).floor() as i64
            ..=((center.x + radius) / side).floor() as i64
        {
            for j in ((center.z - radius) / side).floor() as i64
                ..=((center.z + radius) / side).floor() as i64
            {
                let site = Site::new(self.seed, self.params.mode, family, i, j);
                if site.center.sub(center).len() > radius || self.cached_pond(site, i, j)?.is_none()
                {
                    continue;
                }
                let (x, z) = (site.center.x.round(), site.center.z.round());
                let column = self.water_column(x, z);
                if column.body == body && column.has_water() {
                    out.push((x as i64, z as i64, column));
                    if out.len() == limit {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
    pub(super) fn pond_hit(&self, x: f64, z: f64) -> Result<Option<Hit>, RiverNetError> {
        let (tx, tz) = ((x / 16.).floor() as i64, (z / 16.).floor() as i64);
        let halo = self.params.water.lake_calm_pad_blocks + self.params.water.calm_blocks[1];
        let ponds = self.pond_tiles.get_or_insert((tx, tz), || {
            let lo = Vec2::new(tx as f64 * 16., tz as f64 * 16.);
            let mut out = Vec::new();
            for family in FAMILIES {
                let (side, _, _, _) = family.dimensions(self.params.mode);
                for i in ((lo.x - halo) / side).floor() as i64
                    ..=((lo.x + 16. + halo) / side).floor() as i64
                {
                    for j in ((lo.z - halo) / side).floor() as i64
                        ..=((lo.z + 16. + halo) / side).floor() as i64
                    {
                        let site = Site::new(self.seed, self.params.mode, family, i, j);
                        let d = site.center.sub(lo);
                        let gap =
                            Vec2::new((-d.x).max(d.x - 16.).max(0.), (-d.z).max(d.z - 16.).max(0.));
                        if gap.len() > site.major + halo {
                            continue;
                        }
                        if let Some(pond) = self.cached_pond(site, i, j)? {
                            out.push(pond);
                        }
                    }
                }
            }
            Ok(Arc::new(out))
        })?;
        let mut best: Option<Hit> = None;
        let p = Vec2::new(x, z);
        for pond in ponds.iter() {
            let d = p.sub(pond.site.center);
            let rho = pond.site.rho(p);
            let edge = if rho > 1. {
                d.len() * (1. - 1. / rho)
            } else {
                0.
            };
            if edge > halo {
                continue;
            }
            let hit = Hit {
                body: pond.site.family.body(),
                level: pond.level,
                depth: pond.site.depth,
                rho,
                edge,
            };
            if best.is_none_or(|b| (edge, pond.level) < (b.edge, b.level)) {
                best = Some(hit);
            }
        }
        Ok(best)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contour_matches_inclusion_and_stays_in_cell() {
        for mode in Mode::ALL {
            for f in FAMILIES {
                let s = Site::new(7, mode, f, -2, 3);
                let (side, ..) = f.dimensions(mode);
                assert_eq!(s.rho(s.center), 0.);
                for k in 0..64 {
                    let dir = rotate_unit(
                        Vec2::new(1., 0.),
                        std::f64::consts::TAU * (k as f64 / 64. - 0.5),
                    );
                    assert!((s.rho(s.ring_point(dir, 0.)) - 1.).abs() < 1e-10);
                    let out = s.ring_point(dir, 4.);
                    assert!(s.rho(out) > 1.);
                    assert_eq!(
                        ((out.x / side).floor() as i32, (out.z / side).floor() as i32),
                        (-2, 3)
                    );
                }
            }
        }
    }
    #[test]
    fn wet_tarn_reaches_the_same_column_bed_field_and_abi() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1DevW).unwrap();
        let (x, z) = (-542., 1167.);
        let full = g.sample(x, z).unwrap();
        let col = g.water_column(x, z);
        assert_eq!(
            (col.body, col.level, col.kind_code()),
            (WaterBody::Tarn, 121, 2)
        );
        assert_eq!((col.info_code() >> 12) & 63, 11);
        assert!(col.bed < col.level && col.has_water());
        assert_eq!(full.world.height.surface_y.round() as i32, col.bed);
        assert_eq!(full.world.height.calm, 0.);
        let baseline = g.pond_baseline(Vec2::new(x, z)).unwrap().world.height;
        let (fast, wet, _) = g.pond_terrain(Vec2::new(x, z)).unwrap();
        assert_eq!((fast, wet), (baseline.surface_y, false));
        assert!(baseline.surface_y >= full.world.height.surface_y);
    }
    #[test]
    fn column_is_cut_only_inside_and_never_raised() {
        let p = Params::defaults(GenVersion::V1DevW, Mode::Gameplay);
        let hit = Hit {
            body: WaterBody::Thermokarst,
            level: 84.,
            depth: 2.,
            rho: 0.,
            edge: 0.,
        };
        let h = crate::height::HeightSample {
            surface_y: 88.,
            ..Default::default()
        };
        let middle = hit.apply(h, &p);
        assert_eq!(
            (middle.surface_y, middle.water_y, middle.calm),
            (81., 84., 0.)
        );
        assert_eq!(middle.point_body, Some(hit.body));
        assert_eq!(middle.wet_kind, hydrology::SurfaceKind::Lake);
        assert_eq!(
            hit.apply(
                crate::height::HeightSample {
                    surface_y: 73.,
                    ..h
                },
                &p
            )
            .bed_y,
            73.
        );
        let bank = Hit {
            rho: 1.1,
            edge: 1.,
            ..hit
        }
        .apply(h, &p);
        assert_eq!(bank.surface_y, h.surface_y);
        assert_eq!(bank.point_body, None);
        assert_eq!(bank.point_shore, Some((BedMaterial::Mud, 84)));
    }
}
