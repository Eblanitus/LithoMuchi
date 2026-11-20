//! Candidate locations are keyed to distance along a *reach*, not to a grid
//! over land. Build the cheap geometric list once per plate. Profile class and
//! the expensive S_rl checks are deferred until a candidate is actually near.
use super::*;
use crate::channel::ReachKind;
use crate::drainage::RiverNet;
use std::f64::consts::{FRAC_PI_3, PI};

#[derive(Debug)]
pub(in crate::generator) struct Index(Vec<(usize, Site)>);
impl Index {
    fn build(g: &Generator, net: &RiverNet) -> Self {
        let channels = net.channels();
        let l = g.params.block.l_blocks;
        let spacing = if g.params.mode == Mode::Gameplay {
            400.
        } else {
            1000.
        };
        let prob = if g.params.mode == Mode::Gameplay {
            0.5
        } else {
            0.5 / 1.5
        };
        let salt = subseed(g.seed, "water.oxbow") ^ net.plate_id();
        let mut out = Vec::new();
        for (i, reach) in channels.reaches().iter().enumerate() {
            if !reach.plain || reach.kind != ReachKind::Channel {
                continue;
            }
            let segments = channels.reach_segments(i);
            let mut distance = 0.;
            let mut last_bin = 0;
            let mut peak: Option<(usize, f64)> = None;
            let mut emit = |bin: i64, end: f64, peak: Option<(usize, f64)>| {
                let Some((k, _)) = peak else {
                    return;
                };
                let roll = crate::hash::unit(cell_hash(salt, i as i64, bin, 0));
                if roll >= prob * ((end - bin as f64 * spacing) / spacing).clamp(0., 1.) {
                    return;
                }
                let v = segments[k].a;
                let width = v.core_l * l;
                let outer = segments[k]
                    .b
                    .point
                    .sub(segments[k - 1].a.point)
                    .normalized()
                    .perp()
                    .scale(v.bend.signum());
                let radius = 0.6 * v.meander_amplitude_l * l;
                let tube = 0.4 * width;
                let pivot = v.point.sub(g.spawn_shift).scale(l);
                // Put even the tips of the crescent outside the plain's bank
                // and floodplain; S_rl/ring tests still reject crossings.
                let center = pivot.add(outer.scale(
                    (0.5 + g.params.water.floodplain_w[0]) * width + 6. + tube + 0.5 * radius,
                ));
                let segment = channels.reach_first_segment(i) + k;
                let site = Site {
                    family: Family::Oxbow,
                    roll: 0.,
                    center,
                    axis: outer,
                    major: radius + tube,
                    minor: tube,
                    depth: if g.params.mode == Mode::Gameplay {
                        2.
                    } else {
                        3.
                    },
                    river: Some((pivot, net.plate_id(), i)),
                };
                out.push((segment, site));
            };
            for (k, seg) in segments.iter().enumerate() {
                let bin = (distance / spacing) as i64;
                if bin != last_bin {
                    emit(last_bin, (last_bin + 1) as f64 * spacing, peak.take());
                    last_bin = bin;
                }
                let b = seg.a.bend.abs();
                if k > 0
                    && b > 0.
                    && b >= segments[k - 1].a.bend.abs()
                    && b >= seg.b.bend.abs()
                    && seg.a.meander_amplitude_l >= seg.a.core_l
                    && peak.is_none_or(|(_, old)| b > old)
                {
                    peak = Some((k, b));
                }
                distance += seg.b.point.sub(seg.a.point).len() * l;
            }
            emit(last_bin, distance, peak);
        }
        out.sort_by(|a, b| a.1.center.x.total_cmp(&b.1.center.x).then(a.0.cmp(&b.0)));
        Self(out)
    }
}
impl Site {
    // The candidate centre is the middle of the crescent, not the centre of
    // its circumcircle. Ends are capped so the curve closes without corners.
    fn arc_distance(&self, p: Vec2) -> f64 {
        let r = self.major - self.minor;
        let d = p.sub(self.center).add(self.axis.scale(r));
        let len = d.len();
        if d.dot(self.axis) >= 0.5 * len {
            (len - r).abs()
        } else {
            let tip = rotate_unit(
                self.axis,
                if d.dot(self.axis.perp()) >= 0. {
                    FRAC_PI_3
                } else {
                    -FRAC_PI_3
                },
            );
            d.sub(tip.scale(r)).len()
        }
    }
    pub(super) fn arc_rho(&self, p: Vec2) -> f64 {
        self.arc_distance(p) / self.minor
    }
    pub(super) fn arc_rings(
        &self,
        pad: f64,
        mut visit: impl FnMut(Vec2) -> Result<bool, RiverNetError>,
    ) -> Result<bool, RiverNetError> {
        let r = self.major - self.minor;
        let w = self.minor + pad;
        let origin = self.center.sub(self.axis.scale(r));
        for (length, part) in [
            (2. * FRAC_PI_3 * (r + w), 0),
            (PI * w, 1),
            (2. * FRAC_PI_3 * (r - w).max(0.), 2),
            (PI * w, 3),
        ] {
            let n = (length / 2.).ceil() as usize;
            for k in 0..n {
                let t = k as f64 / n as f64;
                let radial = |angle: f64| rotate_unit(self.axis, angle);
                let p = match part {
                    0 => origin.add(radial((2. * t - 1.) * FRAC_PI_3).scale(r + w)),
                    2 => origin.add(radial((1. - 2. * t) * FRAC_PI_3).scale(r - w)),
                    _ => {
                        let a = if part == 1 { FRAC_PI_3 } else { -FRAC_PI_3 };
                        let offset = if part == 1 { t * PI } else { (t - 1.) * PI };
                        origin
                            .add(radial(a).scale(r))
                            .add(radial(a + offset).scale(w))
                    }
                };
                // An expanded endcap can cross the opposite end for a small
                // radius. Those hidden portions are not on the exterior ring.
                if self.arc_distance(p) + 1e-7 >= w && !visit(p)? {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
}
impl Generator {
    pub(super) fn oxbows_near(
        &self,
        center: Vec2,
        radius: f64,
        halo: f64,
        tile: bool,
        mut visit: impl FnMut(Pond) -> bool,
    ) -> Result<(), RiverNetError> {
        let l = self.params.block.l_blocks;
        let point = self.macro_point(center.x, center.z);
        let support = crate::channel::max_query_radius_l(&self.params);
        // Bounding radius of the largest possible crescent (r + half-width).
        let max_shape = (1.2
            * self
                .params
                .channels
                .meander_plain
                .max(self.params.channels.meander_mountain)
            + 0.4)
            * self.params.channels.core_max_blocks;
        let search = radius + halo;
        let owners = tile.then(|| self.net_owners(point));
        for plate in plates_near(point, support + search / l, self.seed, &self.params) {
            if owners
                .as_ref()
                .is_some_and(|o| o.binary_search(&plate.id).is_err())
            {
                continue;
            }
            let index = match self.oxbows.get_or_build(plate.id, || {
                self.drainage_network(&plate)
                    .map(|net| Index::build(self, &net))
            }) {
                Ok(index) => index,
                Err(RiverNetError::NoOwnedNode { .. }) => continue,
                Err(e) => return Err(e),
            };
            let lo = index
                .0
                .partition_point(|(_, s)| s.center.x < center.x - search - max_shape);
            for &(seg, site) in &index.0[lo..] {
                if site.center.x > center.x + search + max_shape {
                    break;
                }
                let limit = if tile { search + site.major } else { radius };
                if site.center.dist2(center) > limit * limit {
                    continue;
                }
                if let Some(pond) = self.cached_pond(site, plate.id as i64, seg as i64)? {
                    if !visit(pond) {
                        return Ok(());
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crescent_rings_follow_both_faces_and_caps() {
        let s = Site {
            family: Family::Oxbow,
            roll: 0.,
            center: Vec2::new(1., 2.),
            axis: Vec2::new(0., 1.),
            major: 12.,
            minor: 2.,
            depth: 2.,
            river: None,
        };
        assert_eq!(s.arc_rho(s.center), 0.);
        // The centre of the circumcircle is dry, not an elliptical lake.
        assert!(s.arc_rho(s.center.sub(s.axis.scale(10.))) > 1.);
        for pad in [0., 2., 4.] {
            let mut count = 0;
            assert!(
                s.arc_rings(pad, |p| {
                    count += 1;
                    assert!((s.arc_distance(p) - s.minor - pad).abs() < 1e-7);
                    Ok(true)
                })
                .unwrap()
            );
            assert!(count > 20);
        }
    }
    #[test]
    fn oxbow_reaches_chunk_query_with_clay_and_a_distinct_abi_type() {
        let g = Generator::new(42, Mode::Gameplay, GenVersion::V1DevW).unwrap();
        let col = g.water_column(103., -1437.);
        assert_eq!(
            (col.body, col.level, col.bed_mat),
            (WaterBody::Oxbow, 70, BedMaterial::Clay)
        );
        assert!(col.has_water() && col.bed < col.level && !col.wall_exempt);
        assert_eq!((col.info_code() >> 12) & 63, 16);
    }
}
