// Executed in a staged copy of real Horizon 66f3981, not in this workspace.
// It checks the shipped bridge against the genuine Noise/DensityVolume/graph
// contract and the corrected offset encoding (К2, шаг 2 плана готовности).
use horizon_core::density::SamplerContext;
use horizon_core::density::graph::{DensityCompiler, DensityGraph, GraphNode};
use horizon_core::noise::{DensityVolume, NormalNoise};
use litho_core::{Field, GenVersion, Generator, Mode};

#[test]
fn real_seeded_density_and_volume_dispatch() {
    let mut graph = DensityGraph::new();
    let definition = NormalNoise::builder()
        .set_octave_count(0)
        .unwrap()
        .build()
        .unwrap();
    // The datapack's NormalNoise definition controls graph optimization
    // bounds independently of the injected implementation's own Noise::range.
    let i2_definition = NormalNoise::builder()
        .set_base_amplitude(4.0)
        .set_octave_count(1)
        .unwrap()
        .build()
        .unwrap();
    let c = graph
        .register_noise(
            "litospherity:v1-dev/gameplay/continents".into(),
            definition.clone(),
        )
        .unwrap();
    let pv = graph
        .register_noise("litospherity:v1-dev/gameplay/pv".into(), definition.clone())
        .unwrap();
    let offset_noise = graph
        .register_noise(
            "litospherity:v1-dev/gameplay/offset".into(),
            i2_definition,
        )
        .unwrap();
    let smoke = graph
        .register_noise("litospherity:smoke/continents".into(), definition.clone())
        .unwrap();
    let vanilla = graph
        .register_noise("minecraft:plain_test".into(), definition)
        .unwrap();
    let zero = graph.insert(GraphNode::Constant(0.)).unwrap();
    let noise_root = |graph: &mut DensityGraph, noise| {
        graph
            .insert(GraphNode::Noise {
                noise,
                xz_scale: 1.0,
                y_scale: 0.0,
                shift_x: zero,
                shift_y: zero,
                shift_z: zero,
            })
            .unwrap()
    };
    let mut roots = [
        noise_root(&mut graph, c),
        noise_root(&mut graph, pv),
        noise_root(&mut graph, offset_noise),
        noise_root(&mut graph, smoke),
        noise_root(&mut graph, vanilla),
    ];
    let offset_root = roots[2];
    let bound = graph.range(offset_root).unwrap();
    assert!(
        bound.min() <= -2.0 && bound.max() >= 2.0,
        "offset graph range {bound:?}"
    );
    roots[1] = graph
        .register_function("minecraft:overworld/ridges_folded".into(), roots[1])
        .unwrap();
    assert_eq!(
        graph.function("minecraft:overworld/ridges_folded"),
        Some(roots[1])
    );

    let generator = Generator::new(1, Mode::Gameplay, GenVersion::V1Dev).unwrap();
    for legacy in [false, true] {
        let compiler = DensityCompiler::new_seeded(graph.clone(), 1, legacy);
        let injected = compiler.get_noise_sampler(offset_noise).unwrap();
        // NanRange в Horizon не реализует Debug: unwrap() на Result нельзя.
        match horizon_core::noise::Noise::range(injected.as_ref()) {
            Ok([low, high]) => {
                assert_eq!(low, -2.0);
                assert_eq!(high, 2.0);
            }
            Err(_) => panic!("внедрённый offset-шум не вернул диапазон"),
        }
        let samplers: Vec<_> = roots
            .iter()
            .map(|&id| compiler.compile_raw(id).unwrap())
            .collect();
        let xyz = [4859, 300, 1984];
        let x = f64::from(xyz[0]);
        let z = f64::from(xyz[2]);
        let expected = [
            generator.field_column(Field::Continents, x, z) as f32,
            generator.field_column(Field::Pv, x, z) as f32,
            generator.field_column(Field::Offset, x, z) as f32,
            0.8,
            0.0,
        ];
        for (sampler, expected) in samplers.iter().zip(expected) {
            assert_eq!(
                sampler
                    .sample_value(&mut SamplerContext::default(), xyz)
                    .unwrap()
                    .to_bits(),
                expected.to_bits()
            );
            let volume = DensityVolume::new([2, 3, 2], xyz, [2, 4, 3]).unwrap();
            let mut values = vec![0.; volume.size()];
            sampler
                .sample_volume(&mut SamplerContext::default(), &mut values, volume)
                .unwrap();
            for z in 0..2 {
                for x in 0..2 {
                    for y in 0..3 {
                        let p = [xyz[0] + x * 2, xyz[1] + y * 4, xyz[2] + z * 3];
                        let index = volume.index([x as usize, y as usize, z as usize]).unwrap();
                        let single = sampler
                            .sample_value(&mut SamplerContext::default(), p)
                            .unwrap();
                        assert_eq!(values[index].to_bits(), single.to_bits());
                    }
                }
            }
        }
        let offset = compiler.compile_raw(offset_root).unwrap();
        let value = offset
            .sample_value(&mut SamplerContext::default(), xyz)
            .unwrap();
        assert!((-2.0..=2.0).contains(&value));
        assert_eq!(value.to_bits(), expected[2].to_bits());
        // Vanilla's depth gradient at y=80 is 0.375; our offset must encode
        // surface y = 128·(1 + offset), not the old 63.52 + 128·offset.
        let full = generator.sample(x, z).unwrap();
        let surface_from_offset = 128.0 * (1.0 + f64::from(value));
        assert!(
            (surface_from_offset - full.world.height.surface_y).abs() < 1e-3,
            "surface {surface_from_offset} != {:?}",
            full.world.height
        );
        assert!((f64::from(value) - expected[2] as f64).abs() < 1e-6);
        assert_eq!(
            compiler.get_noise_sampler(vanilla).unwrap().get2(22., -42.),
            0.0
        );
    }
}

// T77: the vanilla aquifer lava noise is replaced only when the same graph
// carries our v1-dev noises; a vanilla graph keeps the vanilla sampler.
#[test]
fn aquifer_lava_noise_follows_the_core_only_in_our_world() {
    let lava_definition = || {
        NormalNoise::builder()
            .set_base_amplitude(1.0)
            .set_octave_count(1)
            .unwrap()
            .build()
            .unwrap()
    };
    let plain = NormalNoise::builder()
        .set_octave_count(0)
        .unwrap()
        .build()
        .unwrap();
    let lava_root = |graph: &mut DensityGraph, noise| {
        let zero = graph.insert(GraphNode::Constant(0.)).unwrap();
        // 26.3 noise_settings: aquifers.lava = noise aquifer_lava, scales 1/1.
        graph
            .insert(GraphNode::Noise {
                noise,
                xz_scale: 1.0,
                y_scale: 1.0,
                shift_x: zero,
                shift_y: zero,
                shift_z: zero,
            })
            .unwrap()
    };

    let mut ours = DensityGraph::new();
    let lava = ours
        .register_noise("minecraft:aquifer_lava".into(), lava_definition())
        .unwrap();
    ours.register_noise("litospherity:v1-dev/gameplay/continents".into(), plain.clone())
        .unwrap();
    ours.register_noise("litospherity:v1-dev/gameplay/offset".into(), plain.clone())
        .unwrap();
    let root = lava_root(&mut ours, lava);

    let mut vanilla = DensityGraph::new();
    let vanilla_lava = vanilla
        .register_noise("minecraft:aquifer_lava".into(), lava_definition())
        .unwrap();
    vanilla
        .register_noise("minecraft:plain_test".into(), plain.clone())
        .unwrap();

    let mut v0 = DensityGraph::new();
    let v0_lava = v0
        .register_noise("minecraft:aquifer_lava".into(), lava_definition())
        .unwrap();
    v0.register_noise("litospherity:v0/gameplay/continents".into(), plain)
        .unwrap();

    let generator = Generator::new(1, Mode::Gameplay, GenVersion::V1Dev).unwrap();
    for legacy in [false, true] {
        let compiler = DensityCompiler::new_seeded(ours.clone(), 1, legacy);
        let sampler = compiler.compile_raw(root).unwrap();
        let mut on = 0;
        for x in -40..40 {
            for z in -40..40 {
                // The game samples at aquifer cell indices (x div 64, y div 40, z div 64).
                let value = sampler
                    .sample_value(&mut SamplerContext::default(), [x, -1, z])
                    .unwrap();
                let expected = generator.lava_aquifer(i64::from(x), -1, i64::from(z)) as f32;
                assert_eq!(value.to_bits(), expected.to_bits(), "cell ({x}, -1, {z})");
                on += i32::from(value.abs() > 0.3);
            }
        }
        assert!(on > 0 && on < 6400, "lava cells {on} of 6400");

        for (graph, id) in [(&vanilla, vanilla_lava), (&v0, v0_lava)] {
            let compiler = DensityCompiler::new_seeded(graph.clone(), 1, legacy);
            let noise = compiler.get_noise_sampler(id).unwrap();
            let mut other = false;
            for x in 0..32 {
                let value = noise.get3(f64::from(x) * 0.37, -1.3, 2.1);
                other |= value != 0.0 && value != 0.5;
            }
            assert!(other, "vanilla aquifer lava noise must stay vanilla");
        }
    }
}
