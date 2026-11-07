//! Runs against the ACTUALLY transformed Horizon libraries in CI.
use horizon_core::{
    density::{SamplerContext, graph::DensityCompiler},
    noise::DensityVolume,
    random::XoroshiroPositionalRandom,
};
use horizon_world::{
    registry::BlockStateId,
    worldgen::{
        aquifer::{
            AquiferBlockStates, AquiferConfig, FluidPicker, GlobalFluidPicker, NoiseBasedAquifer,
        },
        density::DensityRegistry,
    },
};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};

fn pack_registry(mode: &str) -> DensityRegistry {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../bundle/packs/litospherity_v0_{mode}/data"));
    let mut noises = BTreeMap::new();
    let mut densities = BTreeMap::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            let parts: Vec<_> = relative.split('/').collect();
            if parts.len() < 4 || parts[1] != "worldgen" {
                continue;
            }
            let id = format!(
                "{}:{}",
                parts[0],
                parts[3..].join("/").trim_end_matches(".json")
            );
            let text = std::fs::read_to_string(path).unwrap();
            match parts[2] {
                "noise" => {
                    noises.insert(id, text);
                }
                "density_function" => {
                    densities.insert(id, text);
                }
                _ => {}
            }
        }
    }
    // 26.3's depth equation; compile it rather than approximating the sampler.
    densities.insert(
        "minecraft:overworld/depth".into(),
        json!({"type":"add",
        "left":{"type":"gradient","axis":"y","from_coordinate":-64,"to_coordinate":320,
        "from_value":1.5,"to_value":-1.5},"right":"minecraft:overworld/offset"})
        .to_string(),
    );
    // Gameplay intentionally has no own offset. Do not silently fabricate one.
    if mode == "gameplay" {
        densities.remove("minecraft:overworld/depth");
    }
    DensityRegistry::from_json(&noises, &densities).unwrap()
}

#[test]
fn scientific_pack_depth_follows_model_height_without_raising_build_limits() {
    let registry = pack_registry("scientific");
    let id = registry.value("minecraft:overworld/depth").unwrap();
    let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), 42, false);
    let sampler = compiler.get_sampler(id).unwrap();
    let mut context = SamplerContext::cached();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden/v0/world-0.3.0.txt");
    let golden = std::fs::read_to_string(golden).unwrap();
    let mut tested = 0;
    for line in golden
        .lines()
        .filter(|l| l.starts_with("42 scientific "))
        .step_by(47)
    {
        let values: Vec<_> = line.split_whitespace().collect();
        let number = |i: usize| f64::from_bits(u64::from_str_radix(values[i], 16).unwrap());
        let (x, z, e) = (number(2) as i32, number(3) as i32, number(9));
        let height = (63.0 + e * if e < 0.0 { 10.0 } else { 28.0 }).clamp(-48.0, 288.0);
        // Exclude random non-integer coordinates in the tail of the golden file.
        if number(2) != f64::from(x) || number(3) != f64::from(z) {
            continue;
        }
        let low = height.floor() as i32 - 1;
        let high = height.ceil() as i32 + 1;
        assert!(sampler.sample_value(&mut context, [x, low, z]).unwrap() > 0.0);
        assert!(sampler.sample_value(&mut context, [x, high, z]).unwrap() < 0.0);
        tested += 1;
    }
    assert!(tested >= 20);
}

#[test]
fn packaged_lava_graph_matches_real_noise_hook_in_scaled_cell_coordinates() {
    for mode in ["gameplay", "scientific"] {
        let registry = pack_registry(mode);
        let id = registry
            .value(&format!("litospherity:v0/{mode}/lava"))
            .unwrap();
        let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), 42, false);
        let sampler = compiler.get_sampler(id).unwrap();
        let noise = horizon_core::litospherity_glue::noise_for_name(
            42,
            &format!("litospherity:v0/{mode}/lava"),
        )
        .unwrap();
        let mut context = SamplerContext::cached();
        for cell in [[0, -1, 0], [-17, -2, 11], [19, -1, -23]] {
            let expected = noise.get3(
                f64::from(cell[0]) * 64.0,
                f64::from(cell[1]) * 40.0,
                f64::from(cell[2]) * 64.0,
            );
            assert_eq!(
                sampler.sample_value(&mut context, cell).unwrap().to_bits(),
                expected.to_bits()
            );
        }
    }
}

struct Blocks;
impl AquiferBlockStates for Blocks {
    fn air(&self) -> BlockStateId {
        BlockStateId(0)
    }
    fn lava(&self) -> BlockStateId {
        BlockStateId(2)
    }
    fn is_air(&self, s: BlockStateId) -> bool {
        s.0 == 0
    }
    fn is_lava(&self, s: BlockStateId) -> bool {
        s.0 == 2
    }
    fn is_water(&self, s: BlockStateId) -> bool {
        s.0 == 1
    }
}

fn fluid_counts(lava_value: f32) -> [usize; 3] {
    let mut registry = DensityRegistry::from_json(&BTreeMap::new(), &BTreeMap::new()).unwrap();
    // A controlled underground aquifer: identical geometry, floodedness and
    // fluid levels in both runs. Only the lava decision differs.
    let config = AquiferConfig::from_json(
        &json!({"barrier":0.0,"fluid_level_floodedness":0.6,
        "fluid_level_spread":0.0,"lava":lava_value,"exclusion":0.0,"surface_level":64.0})
        .to_string(),
        &mut registry,
    )
    .unwrap();
    let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), 42, false);
    let samplers = config.compile(&mut compiler).unwrap();
    let picker = GlobalFluidPicker::new(63, BlockStateId(1), BlockStateId(2), BlockStateId(0));
    assert_eq!(picker.compute([0, -60, 0]).state, BlockStateId(2));
    assert_eq!(picker.compute([0, -30, 0]).state, BlockStateId(1));
    let volume = DensityVolume::new([64, 48, 64], [-32, -52, -32], [1; 3]).unwrap();
    let mut context = SamplerContext::cached();
    let mut aquifer = NoiseBasedAquifer::new(
        samplers,
        picker,
        XoroshiroPositionalRandom::new(1, 2),
        Blocks,
        volume,
        &mut context,
    )
    .unwrap();
    let mut counts = [0; 3];
    for z in -24..24 {
        for x in -24..24 {
            for y in -42..-15 {
                if let Some(state) = aquifer
                    .compute_substance(&mut context, [x, y, z], -1.0)
                    .unwrap()
                {
                    counts[state.0 as usize] += 1;
                }
            }
        }
    }
    counts
}

#[test]
fn native_aquifer_really_selects_lava_above_the_unconditional_bottom_floor() {
    let inactive = fluid_counts(0.0);
    let active = fluid_counts(0.6);
    assert_eq!(inactive[2], 0);
    assert!(inactive[1] > 100);
    assert!(
        active[2] > 100,
        "native aquifer activation produced no lava: {active:?}"
    );
    eprintln!(
        "Native aquifer off={inactive:?}, on={active:?}; queries Y=-42..-16 (above vanilla -54)"
    );
}
