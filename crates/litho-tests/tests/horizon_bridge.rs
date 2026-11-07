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

use litospherity_glue::noise_for_name;
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
    ] {
        assert!(noise_for_name(42, name).is_none(), "{name}");
    }
    let smoke = noise_for_name(42, "litospherity:smoke/continents").unwrap();
    assert_eq!(smoke.get3(1.0, 90.0, -1.0), 0.8);
}

#[test]
fn bulk_matches_scalar_and_preserves_tail() {
    let volume = DensityVolume::new([2, 3, 2], [-7, -64, 11], [4, 8, 4]).unwrap();
    for field in [
        "continents",
        "erosion",
        "ridges",
        "temperature",
        "vegetation",
    ] {
        let noise = noise_for_name(42, &format!("litospherity:v0/gameplay/{field}")).unwrap();
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
            ],
        };
        match litospherity_noise(&ctx, &graph, 0) {
            core::ops::ControlFlow::Break(Ok(noise)) => {
                assert_eq!(noise.get2(0.0, 0.0), 0.8);
            }
            _ => panic!("our noise must replace the vanilla sampler"),
        }
        assert!(matches!(
            litospherity_noise(&ctx, &graph, 1),
            core::ops::ControlFlow::Continue(())
        ));
        assert!(matches!(
            litospherity_noise(&ctx, &graph, 2),
            core::ops::ControlFlow::Continue(())
        ));
    }
}

#[test]
fn lava_bulk_uses_all_three_scaled_block_coordinates() {
    let noise = noise_for_name(42, "litospherity:v0/gameplay/lava").unwrap();
    let generator =
        litho_core::Generator::new(42, litho_core::Mode::Gameplay, litho_core::GenVersion::V0)
            .unwrap();
    let volume = DensityVolume::new([2, 3, 2], [-2, -2, 3], [1, 1, 1]).unwrap();
    let mut buffer = vec![0.25; volume.size() + 1];
    noise
        .add_to_volume(&mut buffer, volume, 64.0, 40.0, 0.75)
        .unwrap();
    let mut index = 0;
    for z in [3.0, 4.0] {
        for x in [-2.0, -1.0] {
            for y in [-2.0, -1.0, 0.0] {
                let sampled = generator.aquifer_lava(x * 64.0, y * 40.0, z * 64.0) as f32;
                assert_eq!(buffer[index].to_bits(), (0.25 + 0.75 * sampled).to_bits());
                index += 1;
            }
        }
    }
    assert_eq!(buffer[volume.size()], 0.25);
}
