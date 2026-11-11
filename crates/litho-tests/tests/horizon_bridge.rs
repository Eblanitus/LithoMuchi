//! Compile the shipped bridge against the pinned Horizon Noise contract.
//! This does not replace a native mixin build or an in-game test.
#![allow(dead_code)]

mod geometry {
    #[derive(Clone, Copy)]
    pub enum Axis {
        X,
        Y,
        Z,
    }
}
mod noise {
    mod volume {
        // Horizon 66f3981, MIT: crates/horizon-core/src/noise/volume.rs.
        include!("support/volume.rs");
    }
    pub use volume::{DensityError, DensityVolume};
    #[derive(Debug)]
    pub struct NanRange;
    include!("support/trait.rs");
}
mod litospherity_glue {
    include!("../../../mod/glue/litospherity_glue.rs.in");
}

use litospherity_glue::{field_for, noise_for_name};
use noise::{DensityError, DensityVolume};

#[test]
fn names_and_smoke() {
    for name in [
        "minecraft:temperature",
        "litospherity:v9/gameplay/continents",
        "litospherity:v1/gameplay/continents",
        "litospherity:v0/unknown/continents",
        "litospherity:v0/gameplay/continents/extra",
        "litospherity:v0/gameplay/offset",
        "litospherity:v0/gameplay/pv",
    ] {
        assert!(noise_for_name(42, name).is_none(), "{name}");
    }
    let smoke = noise_for_name(42, "litospherity:smoke/continents").unwrap();
    assert_eq!(smoke.get3(1.0, 90.0, -1.0), 0.8);

    // К2: v1-dev offset = y/128 − 1; helper and noise must agree.
    let name = "litospherity:v1-dev/gameplay/offset";
    let offset = noise_for_name(42, name).unwrap();
    let value = offset.get3(12.0, 80.0, -34.0);
    let expected = field_for(42, name, 12.0, -34.0).unwrap();
    assert_eq!(value.to_bits(), expected.to_bits());
    assert!((-2.0..=2.0).contains(&value));
    assert_eq!(field_for(42, "litospherity:v0/gameplay/offset", 12.0, -34.0), None);
}

#[test]
fn bulk_matches_scalar_and_preserves_tail() {
    let volume = DensityVolume::new([2, 3, 2], [-7, -64, 11], [4, 8, 4]).unwrap();
    let fields = [
        ("v0", "continents"),
        ("v0", "erosion"),
        ("v0", "ridges"),
        ("v0", "temperature"),
        ("v0", "vegetation"),
        ("v1-dev", "pv"),
        ("v1-dev", "offset"),
    ];
    for (version, field) in fields {
        let name = format!("litospherity:{version}/gameplay/{field}");
        let noise = noise_for_name(42, &name).unwrap();
        let mut buffer = vec![0.25; volume.size() + 2];
        noise
            .add_to_volume(&mut buffer, volume, 0.5, 0.0, 0.75)
            .unwrap();
        let mut index = 0;
        for z in [11.0, 15.0] {
            for x in [-7.0, -3.0] {
                let expected = 0.25 + 0.75 * noise.get3(x * 0.5, 100.0, z * 0.5);
                for _ in 0..3 {
                    assert_eq!(buffer[index].to_bits(), expected.to_bits());
                    index += 1;
                }
            }
        }
        assert_eq!(&buffer[volume.size()..], &[0.25, 0.25]);
        let mut short = [7.0; 2];
        assert_eq!(
            noise.add_to_volume(&mut short, volume, 1.0, 1.0, 1.0),
            Err(DensityError::BufferTooShort)
        );
        assert_eq!(short, [7.0; 2]);
    }
}

mod hook_contract {
    use crate::noise::Noise;
    use std::sync::Arc;

    struct SeededContext {
        seed: i64,
    }
    type NoiseId = usize;
    #[derive(Debug)]
    struct CompileError;
    struct Entry {
        name: String,
    }
    struct DensityGraph {
        entries: Vec<Entry>,
    }
    impl DensityGraph {
        fn noise_entry(&self, id: NoiseId) -> Result<&Entry, CompileError> {
            self.entries.get(id).ok_or(CompileError)
        }
    }

    include!("../../../mod/glue/noise_hook.rs.in");

    #[test]
    fn hook_replaces_our_noise_and_passes_through_others() {
        let ctx = SeededContext { seed: 42 };
        let graph = DensityGraph {
            entries: vec![
                Entry {
                    name: "litospherity:smoke/continents".into(),
                },
                Entry {
                    name: "minecraft:temperature".into(),
                },
                Entry {
                    name: "litospherity:v1-dev/gameplay/offset".into(),
                },
            ],
        };
        match litospherity_noise(&ctx, &graph, 0) {
            core::ops::ControlFlow::Break(Ok(noise)) => {
                assert_eq!(noise.get2(0.0, 0.0), 0.8);
            }
            _ => panic!("наш smoke-шум должен заменить ванильный семплер"),
        }
        assert!(matches!(
            litospherity_noise(&ctx, &graph, 1),
            core::ops::ControlFlow::Continue(())
        ));
        match litospherity_noise(&ctx, &graph, 2) {
            core::ops::ControlFlow::Break(Ok(noise)) => {
                assert!(noise.get2(0.0, 0.0).is_finite());
            }
            _ => panic!("наш offset v1-dev должен заменить ванильный семплер"),
        }
        assert!(matches!(
            litospherity_noise(&ctx, &graph, 3),
            core::ops::ControlFlow::Continue(())
        ));
    }
}
