// Executed in the staged real horizon-world AFTER the real horizon-mixins
// transform, not as part of this workspace. No game data is needed.
// Д-098 (T77, часть 2): the real NoiseBasedAquifer, the real GlobalFluidPicker
// and the three injected hooks. Below y = -54 lava stays only near plate
// junctions in a v1-dev world; a vanilla world keeps lava everywhere.
use std::collections::{BTreeMap, BTreeSet};

use horizon_core::density::SamplerContext;
use horizon_core::litospherity_glue::{deep_lava_by_aquifer, deep_lava_cell};
use horizon_core::noise::DensityVolume;
use horizon_core::random::{PositionalRandom, RandomSource, XoroshiroRandom};
use horizon_world::DATA_VERSION;
use horizon_world::nbt::Tag;
use horizon_world::registry::{BlockRegistry, BlockStateId, BlockStateJsonCodec};
use horizon_world::worldgen::aquifer::{
    AquiferConfig, AquiferField, AquiferSamplers, GlobalFluidPicker, NoiseBasedAquifer,
    RegistryAquiferBlockStates,
};
use horizon_world::worldgen::density::DensityRegistry;
use horizon_world::worldgen::material_condition::{MaterialConditionRegistry, MaterialRuleLibrary};

const SEED: i64 = 42;
const AIR: BlockStateId = BlockStateId(0);
const LAVA: BlockStateId = BlockStateId(1);
const WATER: BlockStateId = BlockStateId(2);

// 26.3 keeps `lava` as a plain noise node; the other five roots are constants
// here: no barrier noise, dry everywhere, excluded everywhere (the worst case
// for junction lava: the deep-dark exclusion under mountains), surface at 100.
const AQUIFERS: &str = r#"{
  "barrier": 0.0,
  "fluid_level_floodedness": -1.0,
  "fluid_level_spread": 0.0,
  "lava": {"type": "minecraft:noise", "noise": "minecraft:aquifer_lava", "xz_scale": 1.0, "y_scale": 1.0},
  "exclusion": 1.0,
  "surface_level": 100.0
}"#;
const NOISE: &str = r#"{ "base_octave": 0, "octave_count": 1 }"#;

fn registry() -> BlockRegistry {
    let names = ["minecraft:air", "minecraft:lava", "minecraft:water", "minecraft:stone"];
    BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        (
            "states",
            Tag::List(
                names
                    .iter()
                    .map(|name| Tag::compound([("id", Tag::String((*name).into()))]))
                    .collect(),
            ),
        ),
        (
            "defaults",
            Tag::compound(
                names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| (*name, Tag::Int(index as i32))),
            ),
        ),
    ]))
    .unwrap()
}

/// The world's aquifer roots through the injected `compile_aquifer`.
fn world(registry: &BlockRegistry, ours: bool) -> AquiferSamplers {
    let mut noises = BTreeMap::from([("minecraft:aquifer_lava".to_owned(), NOISE.to_owned())]);
    if ours {
        for field in ["continents", "offset"] {
            noises.insert(
                format!("litospherity:v1-dev/gameplay/{field}"),
                NOISE.to_owned(),
            );
        }
    }
    let mut densities = DensityRegistry::from_json(&noises, &BTreeMap::new()).unwrap();
    let config = AquiferConfig::from_json(AQUIFERS, &mut densities).unwrap();
    let blocks = BlockStateJsonCodec::new(registry, BTreeSet::new()).unwrap();
    let mut library = MaterialRuleLibrary::from_json(
        &BTreeMap::new(),
        &MaterialConditionRegistry::default(),
        &blocks,
        densities,
        SEED,
        false,
    )
    .unwrap();
    library.compile_aquifer(config).unwrap()
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Count {
    /// Blocks below y = -54: lava, air, water, solid barrier.
    deep: [u32; 4],
    /// Blocks from y = -54 to y = -30: lava, air, water, solid barrier.
    above: [u32; 4],
}

/// One chunk exactly as the generator builds it: new samplers binding, the
/// real global picker, the real registry block states, air everywhere.
fn chunk(registry: &BlockRegistry, samplers: &AquiferSamplers, min: [i32; 2]) -> Count {
    let volume = DensityVolume::new([16, 384, 16], [min[0], -64, min[1]], [1, 1, 1]).unwrap();
    let mut context = SamplerContext::cached();
    println!("progress: chunk {min:?} begins");
    let mut aquifer = NoiseBasedAquifer::new(
        AquiferSamplers::new(AquiferField::ALL.map(|field| samplers.get(field).clone())),
        GlobalFluidPicker::new(63, WATER, LAVA, AIR),
        XoroshiroRandom::new(SEED)
            .fork_positional()
            .from_hash_of("minecraft:aquifer")
            .fork_positional(),
        RegistryAquiferBlockStates::new(registry).unwrap(),
        volume,
        &mut context,
    )
    .unwrap();
    println!("progress: aquifer built");
    let mut count = Count::default();
    for x in min[0]..min[0] + 16 {
        for z in min[1]..min[1] + 16 {
            for y in (-64..=-30).rev() {
                let state = aquifer
                    .compute_substance(&mut context, [x, y, z], -1.0)
                    .unwrap();
                let slot = match state {
                    Some(LAVA) => 0,
                    Some(AIR) => 1,
                    Some(WATER) => 2,
                    None => 3,
                    Some(other) => panic!("unexpected block {other:?}"),
                };
                if y < -54 {
                    count.deep[slot] += 1;
                } else {
                    count.above[slot] += 1;
                }
            }
        }
    }
    count
}

/// A 64x64 cell at a plate junction on land, and one far from any junction.
fn cells() -> ([i64; 2], [i64; 2]) {
    let mut near = None;
    let mut far = None;
    for cx in -64..64 {
        for cz in -64..64 {
            let (lambda, land) = deep_lava_cell(SEED, "gameplay", cx, cz).unwrap();
            if near.is_none() && lambda >= 0.5 && land {
                near = Some([cx, cz]);
            }
            if far.is_none() && lambda <= 0.2 {
                far = Some([cx, cz]);
            }
            if let (Some(near), Some(far)) = (near, far) {
                return (near, far);
            }
        }
    }
    (near.expect("junction cell"), far.expect("far cell"))
}

#[test]
fn lava_below_minus_54_stays_only_at_plate_junctions() {
    let registry = registry();
    let started = std::time::Instant::now();
    let (near, far) = cells();
    println!("progress: cells {near:?} {far:?} at {:?}", started.elapsed());
    // The middle chunk of the cell: its aquifer grid stays inside the cell.
    let at = |cell: [i64; 2]| [(cell[0] * 64 + 16) as i32, (cell[1] * 64 + 16) as i32];
    let deep_blocks = 16 * 16 * 10;

    let ours = world(&registry, true);
    let vanilla = world(&registry, false);
    println!("progress: worlds at {:?}", started.elapsed());

    let junction = chunk(&registry, &ours, at(near));
    assert!(deep_lava_by_aquifer(), "our aquifer must mark the thread");
    println!("v1-dev, junction {near:?}: {junction:?}");

    let remote = chunk(&registry, &ours, at(far));
    println!("v1-dev, far {far:?}: {remote:?}");

    // The same thread now builds a vanilla world: the mark must be dropped.
    let plain_near = chunk(&registry, &vanilla, at(near));
    assert!(!deep_lava_by_aquifer(), "vanilla aquifer must clear the mark");
    let plain_far = chunk(&registry, &vanilla, at(far));
    println!("vanilla, junction cell: {plain_near:?}");
    println!("vanilla, far cell: {plain_far:?}");

    // Vanilla: everything open below -54 is lava, nothing above is.
    for plain in [&plain_near, &plain_far] {
        assert_eq!(plain.deep, [deep_blocks, 0, 0, 0]);
        assert_eq!(plain.above[0], 0);
    }
    // Our world far from junctions: no lava and no water below -54 at all.
    assert_eq!(remote.deep, [0, deep_blocks, 0, 0]);
    assert_eq!(remote.above[0], 0);
    // Our world at a junction: lava as in vanilla, up to -54 and not higher.
    assert!(
        junction.deep[0] * 10 >= deep_blocks * 9,
        "junction lava {} of {deep_blocks}",
        junction.deep[0]
    );
    assert_eq!(junction.deep[2], 0, "no water below -54 at a junction");
    assert_eq!(junction.above[0], 0, "no lava at or above -54");

    // And back: after a vanilla chunk our world still works on this thread.
    assert_eq!(chunk(&registry, &ours, at(far)), remote);
    assert_eq!(chunk(&registry, &ours, at(near)), junction);
}
