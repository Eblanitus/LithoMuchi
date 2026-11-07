//! Scientific terrain within the EXISTING Overworld build range (-64..320).
//! Offset is coupled to the 26.3 depth gradient: depth = 1 - y/128 + offset.
//! No change to dimension type, build height, sea level, or gameplay fields.

pub const SEA_LEVEL: f64 = 63.0;
pub const MIN_SURFACE_Y: f64 = -48.0;
pub const MAX_SURFACE_Y: f64 = 288.0;

/// Vertical compression is necessary to fit planetary kilometres in 384 blocks.
/// Preserve sea level/sign, deep trenches and continental plateaus explicitly.
pub fn surface_y(e_km: f64) -> f64 {
    if !e_km.is_finite() {
        return SEA_LEVEL;
    }
    let blocks_per_km = if e_km < 0.0 { 10.0 } else { 28.0 };
    (SEA_LEVEL + blocks_per_km * e_km).clamp(MIN_SURFACE_Y, MAX_SURFACE_Y)
}

pub fn offset(e_km: f64) -> f64 {
    surface_y(e_km) / 128.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn own_height_is_monotone_bounded_and_preserves_sea_level() {
        assert_eq!(surface_y(0.0), 63.0);
        assert_eq!(surface_y(-4.5), 18.0);
        assert_eq!(surface_y(-8.0), -17.0);
        assert_eq!(surface_y(2.0), 119.0);
        assert_eq!(surface_y(7.0), 259.0);
        let mut previous = MIN_SURFACE_Y;
        for i in -2000..=2000 {
            let e = f64::from(i) / 100.0;
            let y = surface_y(e);
            assert!((MIN_SURFACE_Y..=MAX_SURFACE_Y).contains(&y));
            assert!(y >= previous);
            assert!((1.0 - y / 128.0 + offset(e)).abs() < 1e-12);
            previous = y;
        }
        assert_eq!(surface_y(f64::NAN), SEA_LEVEL);
    }
}
