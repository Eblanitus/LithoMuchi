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
    // End of the lower tread. Normally just the lip itself; a hanging
    // tributary has a short pool at receiver height before its final knot.
    lower_end: f64,
    hanging: bool,
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
        hanging_mouth: bool,
    ) -> Self {
        let mut lips = Vec::new();
        let mut prev = 0;
        for next in 1..levels.len() {
            let hanging = hanging_mouth && next == levels.len() - 1;
            if !hanging && targets[next].is_none() {
                continue;
            }
            let first = prev;
            prev = next;
            let up = levels[first].floor();
            let down = levels[next].floor();
            let drop = up - down;
            if drop < w.fall_min_drop_blocks
                || (!hanging
                    && (classes[next - 1].valley == ValleyClass::Plain
                        || classes[first].valley == ValleyClass::Plain
                        || (first > 0 && classes[first - 1].flow == FlowClass::Cascade)))
            {
                continue;
            }
            let span = stations[next] - stations[first];
            if span <= 0. {
                continue;
            }
            let radius = width(next);
            let end = if hanging {
                stations[next] - radius.min(span * 0.5)
            } else {
                stations[next]
            };
            let count = (drop / w.fall_max_height_blocks.max(1.)).ceil() as usize;
            let mut start = stations[first];
            let mut upper = up;
            for k in 1..=count {
                let t = k as f64 / count as f64;
                let at = if k == count {
                    end
                } else {
                    stations[first] + t * (end - stations[first])
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
                    radius,
                    hanging,
                    lower_end: if k == count && hanging {
                        stations[next]
                    } else {
                        at
                    },
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
            if lip.hanging && station <= lip.lower_end {
                level = lip.down;
            }
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

/// A geometric waterfall in a reach profile (not proof of an exposed block face).
#[derive(Debug)]
pub struct FallSite {
    pub center: crate::Vec2,
    pub height: u32,
    pub upper_level: i32,
    pub lower_level: i32,
    pub valley: ValleyClass,
    pub hanging: bool,
}
#[derive(Debug, Default)]
pub struct ProfileSurvey {
    pub lips: Vec<FallSite>,
    /// Horizontal arc length in blocks, split at both profile knots and the
    /// survey disk boundary. Order: calm, riffle, rapid, cascade.
    pub slope_length: [f64; 4],
}

fn disk_interval(
    a: crate::Vec2,
    b: crate::Vec2,
    center: crate::Vec2,
    radius: f64,
) -> Option<(f64, f64)> {
    let d = b.sub(a);
    let len2 = d.dot(d);
    if len2 == 0. {
        return None;
    }
    let rel = center.sub(a);
    let along = rel.dot(d) / len2;
    let perpendicular = rel.sub(d.scale(along));
    let margin = radius * radius - perpendicular.dot(perpendicular);
    if margin <= 0. {
        return None;
    }
    let span = (margin / len2).sqrt();
    let lo = (along - span).max(0.);
    let hi = (along + span).min(1.);
    (lo < hi).then_some((lo, hi))
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
        Ok(self
            .river_profile_survey(plate, center, radius)?
            .lips
            .into_iter()
            .map(|lip| lip.center)
            .collect())
    }

    /// Axis lengths and geometric lips on channel reaches belonging to this plate.
    /// `center` and `radius` are in block coordinates relative to spawn.
    pub fn river_profile_survey(
        &self,
        plate: &crate::plates::Plate,
        center: crate::Vec2,
        radius: f64,
    ) -> Result<ProfileSurvey, crate::drainage::RiverNetError> {
        let mut out = ProfileSurvey::default();
        if !self.params.version.is_water_update() || !radius.is_finite() || radius < 0. {
            return Ok(out);
        }
        let net = self.drainage_network(plate)?;
        let mut levels = None;
        let l = self.params.block.l_blocks;
        let q = self.macro_point(center.x, center.z);
        for i in 0..net.channels().reaches().len() {
            let segs = net.channels().reach_segments(i);
            if net.channels().reaches()[i].kind != crate::channel::ReachKind::Channel
                || !segs
                    .iter()
                    .any(|s| disk_interval(s.a.point, s.b.point, q, radius / l).is_some())
            {
                continue;
            }
            let levels = match &levels {
                Some(v) => v,
                None => levels.insert(self.w_plate_levels(plate, &net)?),
            };
            let p = self.w_reach_profile(levels, &net, i)?;
            for (k, s) in segs.iter().enumerate() {
                let Some((lo, hi)) = disk_interval(s.a.point, s.b.point, q, radius / l) else {
                    continue;
                };
                let length = s.b.point.sub(s.a.point).len() * l;
                let first = p.starts[k];
                let count = p.starts[k + 1] - first;
                for j in 0..count {
                    let portion = (hi.min((j + 1) as f64 / count as f64)
                        - lo.max(j as f64 / count as f64))
                    .max(0.);
                    let class = p.classes[first + j].flow.code() as usize;
                    out.slope_length[class] += length * portion;
                }
            }
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
                    let upper = p
                        .stations
                        .partition_point(|&s| s < lip.at)
                        .saturating_sub(1);
                    out.lips.push(FallSite {
                        center: v,
                        height: (lip.up - lip.down) as u32,
                        upper_level: lip.up as i32,
                        lower_level: lip.down as i32,
                        valley: p.classes[upper].valley,
                        hanging: lip.hanging,
                    });
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
    fn survey_clips_chords_not_centres() {
        use crate::Vec2;
        let a = Vec2::new(-8., 0.);
        let b = Vec2::new(8., 0.);
        assert_eq!(disk_interval(a, b, Vec2::default(), 4.), Some((0.25, 0.75)));
        assert_eq!(disk_interval(a, b, Vec2::new(8., 0.), 4.), Some((0.75, 1.)));
        assert_eq!(disk_interval(a, b, Vec2::new(0., 4.), 4.), None);
        assert_eq!(disk_interval(a, a, Vec2::default(), 20.), None);
    }
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
            Falls::build(
                &stations,
                &targets,
                &levels,
                classes,
                |_| 8.,
                &p.water,
                false,
            )
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
    fn hanging_mouth_reaches_receiver_before_its_endpoint_without_grading_the_head() {
        let p = crate::Params::defaults(crate::GenVersion::V1DevW, crate::Mode::Gameplay);
        let st = [0., 4., 5., 8.];
        let level = [100., 100., 100., 79.];
        let t = [Some(100.), Some(100.), None, Some(100.)];
        let c = [SegmentClass {
            valley: ValleyClass::GlacialU,
            flow: FlowClass::Cascade,
            ..Default::default()
        }; 3];
        let f = Falls::build(&st, &t, &level, &c, |_| 8., &p.water, true);
        assert_eq!(f.0.len(), 1);
        assert_eq!(
            f.sample(2., 0., 100., FlowClass::Cascade),
            (100., FlowClass::Cascade, 0.)
        );
        assert_eq!(
            f.sample(5.75, 0., 94., FlowClass::Cascade),
            (100., FlowClass::FallTop, 0.)
        );
        assert_eq!(
            f.sample(6., 0., 93., FlowClass::Cascade),
            (79., FlowClass::Plunge, 7.)
        );
        assert_eq!(f.sample(8., 0., 79., FlowClass::Cascade).0, 79.);
        assert!(Falls::build(&st, &t, &level, &c, |_| 8., &p.water, false).is_empty());
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
                false,
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
