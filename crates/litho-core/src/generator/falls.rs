//! Local discontinuities on a measured reach. Knots inserted for channel
//! geometry are not new measurements: subdividing a segment must not turn a
//! single cliff into several ramps too small to qualify as a fall.
use crate::params::WaterUpdateParams;
use crate::water_profile::{FlowClass, SegmentClass, ValleyClass};

#[derive(Debug)]
struct Lip {
    start: f64,
    at: f64,
    up: f64,
    down: f64,
    radius: f64,
}
#[derive(Debug, Default)]
pub(super) struct Falls(Vec<Lip>);

impl Falls {
    pub(super) fn build(
        stations: &[f64],
        targets: &[Option<f64>],
        levels: &[f64],
        classes: &[SegmentClass],
        width: impl Fn(usize) -> f64,
        w: &WaterUpdateParams,
    ) -> Self {
        let mut lips = Vec::new();
        let mut prev = 0;
        for next in 1..levels.len() {
            if targets[next].is_none() {
                continue;
            }
            let first = prev;
            prev = next;
            let up = levels[first].floor();
            let down = levels[next].floor();
            let drop = up - down;
            if drop < w.fall_min_drop_blocks
                || classes[next - 1].valley == ValleyClass::Plain
                || classes[first].valley == ValleyClass::Plain
                || (first > 0 && classes[first - 1].flow == FlowClass::Cascade)
            {
                continue;
            }
            let span = stations[next] - stations[first];
            if span <= 0. {
                continue;
            }
            let count = (drop / w.fall_max_height_blocks.max(1.)).ceil() as usize;
            let mut start = stations[first];
            let mut upper = up;
            for k in 1..=count {
                let t = k as f64 / count as f64;
                let at = if k == count {
                    stations[next]
                } else {
                    stations[first] + t * span
                };
                let lower = if k == count {
                    down
                } else {
                    up - (drop * t).floor()
                };
                lips.push(Lip {
                    start,
                    at,
                    up: upper,
                    down: lower,
                    radius: width(next),
                });
                start = at;
                upper = lower;
            }
        }
        Self(lips)
    }
    /// Return mirror, flow and the minimum depth of this part of the pool.
    /// Only the downstream half of the bowl may undercut the lower bank.
    pub(super) fn sample(
        &self,
        station: f64,
        cross: f64,
        mut level: f64,
        mut flow: FlowClass,
    ) -> (f64, FlowClass, f64) {
        let n = self.0.partition_point(|lip| lip.at <= station);
        if let Some(lip) = self.0.get(n).filter(|lip| station >= lip.start) {
            level = lip.up;
            if lip.at - station <= 2. {
                flow = FlowClass::FallTop;
            }
        }
        if let Some(lip) = n.checked_sub(1).map(|k| &self.0[k]) {
            let d2 = (station - lip.at).powi(2) + cross * cross;
            if d2 < lip.radius * lip.radius && level == lip.down {
                let depth = ((lip.up - lip.down) / 3.).clamp(2., 8.)
                    * (1. - d2 / (lip.radius * lip.radius));
                // A short intermediate tread can belong to a plunge basin and
                // also supply the next falling curtain. Preserve its lip flag.
                if flow != FlowClass::FallTop {
                    flow = FlowClass::Plunge;
                }
                return (level, flow, depth);
            }
        }
        (level, flow, 0.)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl super::Generator {
    /// Geometric lip centres, in block coordinates, for sandbox inspection.
    /// A profile discontinuity is not by itself proof that a block is wet.
    pub fn waterfall_lips(
        &self,
        plate: &crate::plates::Plate,
        center: crate::Vec2,
        radius: f64,
    ) -> Result<Vec<crate::Vec2>, crate::drainage::RiverNetError> {
        let mut out = vec![];
        if !self.params.version.is_water_update() || !radius.is_finite() || radius < 0. {
            return Ok(out);
        }
        let net = self.drainage_network(plate)?;
        let mut levels = None;
        let l = self.params.block.l_blocks;
        let q = self.macro_point(center.x, center.z);
        for i in 0..net.channels().reaches().len() {
            let segs = net.channels().reach_segments(i);
            if !segs.iter().any(|s| {
                let lo_x = s.a.point.x.min(s.b.point.x) - radius / l;
                let hi_x = s.a.point.x.max(s.b.point.x) + radius / l;
                let lo_z = s.a.point.z.min(s.b.point.z) - radius / l;
                let hi_z = s.a.point.z.max(s.b.point.z) + radius / l;
                (lo_x..=hi_x).contains(&q.x) && (lo_z..=hi_z).contains(&q.z)
            }) {
                continue;
            }
            let levels = match &levels {
                Some(v) => v,
                None => levels.insert(self.w_plate_levels(plate, &net)?),
            };
            let p = self.w_reach_profile(levels, &net, i)?;
            for lip in &p.falls.0 {
                let k = p
                    .starts
                    .partition_point(|&s| p.stations[s] <= lip.at)
                    .saturating_sub(1)
                    .min(segs.len() - 1);
                let a = p.stations[p.starts[k]];
                let b = p.stations[p.starts[k + 1]];
                if b <= a {
                    continue;
                }
                let seg = segs[k];
                let v = seg
                    .a
                    .point
                    .add(seg.b.point.sub(seg.a.point).scale((lip.at - a) / (b - a)))
                    .sub(self.spawn_shift)
                    .scale(l);
                if v.dist2(center) <= radius * radius {
                    out.push(v);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_measured_cliff_holds_its_upper_level_until_the_lip() {
        let p = crate::Params::defaults(crate::GenVersion::V1DevW, crate::Mode::Gameplay);
        let stations = [0., 1., 2., 3., 4., 8.];
        let targets = [Some(120.), None, None, None, Some(96.), Some(96.)];
        let levels = [120., 114., 108., 102., 96., 96.];
        let mut classes = [SegmentClass {
            valley: ValleyClass::Upland,
            ..Default::default()
        }; 5];
        let make = |classes: &[SegmentClass]| {
            Falls::build(&stations, &targets, &levels, classes, |_| 8., &p.water)
        };
        let falls = make(&classes);
        assert_eq!(falls.0.len(), 1);
        assert_eq!(
            falls.sample(2.5, 0., 105., FlowClass::Rapid),
            (120., FlowClass::FallTop, 0.)
        );
        assert_eq!(
            falls.sample(4., 0., 96., FlowClass::Calm),
            (96., FlowClass::Plunge, 8.)
        );
        assert_eq!(
            falls.sample(4., 8., 96., FlowClass::Rapid),
            (96., FlowClass::Rapid, 0.)
        );
        classes.fill(SegmentClass::default());
        assert!(make(&classes).is_empty());
    }
    #[test]
    fn actual_face_supplies_adjacent_river_columns_and_packed_flow() {
        let g =
            crate::Generator::new(42, crate::Mode::Gameplay, crate::GenVersion::V1DevW).unwrap();
        let upper = g.water_column(-44565., 6791.);
        let lower = g.water_column(-44565., 6792.);
        assert_eq!((upper.level, lower.level), (71, 68));
        assert_eq!(
            (upper.flow, lower.flow),
            (FlowClass::FallTop, FlowClass::Plunge)
        );
        assert_eq!((upper.info_code() & 15, lower.info_code() & 15), (4, 5));
        assert_eq!(lower.bed_mat, crate::water_profile::BedMaterial::Cobble);
        assert!(lower.bed <= lower.level - 2 && upper.bed < upper.level);
    }
    #[test]
    fn splits_tall_cliffs_but_not_where_a_cascade_already_enters() {
        let p = crate::Params::defaults(crate::GenVersion::V1DevW, crate::Mode::Gameplay);
        let mut classes = [SegmentClass {
            valley: ValleyClass::Upland,
            ..Default::default()
        }; 2];
        let make = |classes: &[SegmentClass]| {
            Falls::build(
                &[0., 4., 8.],
                &[Some(143.), Some(143.), Some(90.)],
                &[143., 143., 90.],
                classes,
                |_| 5.,
                &p.water,
            )
        };
        let falls = make(&classes);
        assert_eq!(falls.0.len(), 3);
        assert_eq!(falls.0.last().unwrap().down, 90.);
        assert!(falls.0.iter().all(|s| s.up - s.down <= 24.));
        classes[0].flow = FlowClass::Cascade;
        assert!(make(&classes).is_empty());
    }
}
