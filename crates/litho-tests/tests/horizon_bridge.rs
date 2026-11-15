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
mod density {
    // Horizon 66f3981: контракт DensitySampler из density/mod.rs (сигнатуры).
    use crate::noise::DensityVolume;
    #[derive(Default)]
    pub struct SamplerContext {
        _samples: u64,
    }
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum SampleError {
        BufferTooShort,
    }
    pub trait DensitySampler: std::fmt::Debug + Send + Sync {
        fn sample_value(
            &self,
            context: &mut SamplerContext,
            position: [i32; 3],
        ) -> Result<f32, SampleError>;
        fn sample_volume(
            &self,
            context: &mut SamplerContext,
            buffer: &mut [f32],
            volume: DensityVolume,
        ) -> Result<(), SampleError> {
            if buffer.len() < volume.size() {
                return Err(SampleError::BufferTooShort);
            }
            let [sx, sy, sz] = volume.sizes();
            let minimum = volume.minimum();
            let steps = volume.steps();
            let at = |axis: usize, i: usize| minimum[axis] + i as i32 * steps[axis];
            let mut index = 0;
            for z in 0..sz {
                for x in 0..sx {
                    for y in 0..sy {
                        let position = [at(0, x), at(1, y), at(2, z)];
                        buffer[index] = self.sample_value(context, position)?;
                        index += 1;
                    }
                }
            }
            Ok(())
        }
    }
}
mod litospherity_glue {
    include!("../../../mod/glue/litospherity_glue.rs.in");
}
/// Помощники horizon-world обращаются к клею как к `horizon_core::…`.
mod horizon_core {
    pub mod litospherity_glue {
        pub use crate::litospherity_glue::*;
    }
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
    assert_eq!(
        field_for(42, "litospherity:v0/gameplay/offset", 12.0, -34.0),
        None
    );
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
        noise_names: std::collections::HashMap<String, NoiseId>,
    }
    fn graph_of(names: &[&str]) -> DensityGraph {
        DensityGraph {
            entries: names.iter().map(|n| Entry { name: (*n).into() }).collect(),
            noise_names: names
                .iter()
                .enumerate()
                .map(|(i, n)| ((*n).to_owned(), i))
                .collect(),
        }
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
        let graph = graph_of(&[
            "litospherity:smoke/continents",
            "minecraft:temperature",
            "litospherity:v1-dev/gameplay/offset",
        ]);
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

    #[test]
    fn aquifer_lava_is_ours_only_in_a_v1_dev_world() {
        let ctx = SeededContext { seed: 42 };
        let lava = "minecraft:aquifer_lava";
        // Наш мир v1-dev: шум лавы подменён и совпадает с ядром.
        let graph = graph_of(&[
            lava,
            "litospherity:v1-dev/gameplay/continents",
            "litospherity:v1-dev/gameplay/offset",
            "litospherity:smoke/continents",
            "minecraft:temperature",
        ]);
        let generator = litho_core::Generator::new(
            42,
            litho_core::Mode::Gameplay,
            litho_core::GenVersion::V1Dev,
        )
        .unwrap();
        match litospherity_noise(&ctx, &graph, 0) {
            core::ops::ControlFlow::Break(Ok(noise)) => {
                let mut on = 0;
                for x in -40..40 {
                    for z in -40..40 {
                        let v = noise.get3(f64::from(x), -1.0, f64::from(z));
                        let expected =
                            generator.lava_aquifer(i64::from(x), -1, i64::from(z)) as f32;
                        assert_eq!(v.to_bits(), expected.to_bits());
                        assert!(v == 0.0 || v == 0.5);
                        on += i32::from(v > 0.3);
                    }
                }
                assert!(on > 0 && on < 6400, "лавовых ячеек {on} из 6400");
            }
            _ => panic!("в мире v1-dev шум лавы аквиферов должен быть нашим"),
        }
        // Ванильный мир, мир v0 и граф с двумя режимами: лава ванильная.
        for names in [
            vec![lava, "minecraft:temperature"],
            vec![lava, "litospherity:v0/gameplay/continents"],
            vec![lava, "litospherity:smoke/continents"],
            vec![
                lava,
                "litospherity:v1-dev/gameplay/continents",
                "litospherity:v1-dev/scientific/continents",
            ],
        ] {
            let graph = graph_of(&names);
            assert!(
                matches!(
                    litospherity_noise(&ctx, &graph, 0),
                    core::ops::ControlFlow::Continue(())
                ),
                "{names:?}"
            );
        }
    }
}

/// Д-098: поля аквифера и три помощника horizon-world на подставных типах.
mod aquifer_contract {
    use crate::density::{DensitySampler, SampleError, SamplerContext};
    use crate::horizon_core;
    use crate::litospherity_glue as glue;
    use crate::noise::{DensityVolume, Noise};
    use std::sync::Arc;

    /// Узел графа «шум с масштабами 1/1»: так 26.3 задаёт поле лавы.
    #[derive(Debug)]
    struct NoiseNode(Arc<dyn Noise>);
    impl DensitySampler for NoiseNode {
        fn sample_value(&self, _: &mut SamplerContext, p: [i32; 3]) -> Result<f32, SampleError> {
            Ok(self
                .0
                .get3(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])))
        }
    }
    #[derive(Debug)]
    struct Constant(f32);
    impl DensitySampler for Constant {
        fn sample_value(&self, _: &mut SamplerContext, _: [i32; 3]) -> Result<f32, SampleError> {
            Ok(self.0)
        }
    }

    const SEED: i64 = 42;
    /// Ванильные значения подставных полей: барьер, затопленность, разброс,
    /// (лава), исключение, поверхность.
    const VANILLA: [f32; 6] = [0.125, -0.25, 0.0, 0.0, 0.75, 100.0];

    fn fields(lava: Arc<dyn DensitySampler>) -> [Arc<dyn DensitySampler>; 6] {
        std::array::from_fn(|index| {
            if index == glue::AQUIFER_LAVA {
                lava.clone()
            } else {
                Arc::new(Constant(VANILLA[index])) as Arc<dyn DensitySampler>
            }
        })
    }

    fn our_lava() -> Arc<dyn DensitySampler> {
        let names = [
            "minecraft:aquifer_lava".to_owned(),
            "litospherity:v1-dev/gameplay/offset".to_owned(),
        ];
        Arc::new(NoiseNode(
            glue::aquifer_lava_for(SEED, names.iter()).unwrap(),
        ))
    }

    fn generator() -> litho_core::Generator {
        litho_core::Generator::new(
            SEED,
            litho_core::Mode::Gameplay,
            litho_core::GenVersion::V1Dev,
        )
        .unwrap()
    }

    /// Ячейка 64×64 у стыка на суше и ячейка вдали от стыка.
    fn cells(generator: &litho_core::Generator) -> ([i64; 2], [i64; 2]) {
        let mut near = None;
        let mut far = None;
        for cx in -64..64 {
            for cz in -64..64 {
                if near.is_none() && generator.lava_deep(cx, cz) && generator.lava_deep_land(cx, cz)
                {
                    near = Some([cx, cz]);
                }
                if far.is_none() && generator.lava_cell(cx, cz) <= 0.2 {
                    far = Some([cx, cz]);
                }
            }
        }
        (near.expect("ячейка у стыка"), far.expect("ячейка вдали"))
    }

    #[test]
    fn our_world_gets_wrapped_fields_and_vanilla_world_does_not() {
        // Ванильное поле лавы: мир не наш, поля остаются как были.
        assert!(glue::aquifer_samplers(&fields(Arc::new(Constant(0.0)))).is_none());

        let generator = generator();
        let (near, far) = cells(&generator);
        let wrapped = glue::aquifer_samplers(&fields(our_lava())).expect("мир v1-dev");
        let mut context = SamplerContext::default();
        let mut at = |index: usize, p: [i64; 3]| {
            wrapped[index]
                .sample_value(&mut context, p.map(|v| v as i32))
                .unwrap()
        };
        let block = |cell: [i64; 2], y: i64| [cell[0] * 64 + 20, y, cell[1] * 64 + 20];

        // Ниже −54: у стыка залито до общего уровня, вдали сухо.
        assert_eq!(at(glue::AQUIFER_FLOODEDNESS, block(near, -60)), 1.0);
        assert_eq!(at(glue::AQUIFER_FLOODEDNESS, block(far, -60)), -1.0);
        // От −54 до −41: у стыка свой уровень ячейки, вдали ваниль.
        assert_eq!(at(glue::AQUIFER_FLOODEDNESS, block(near, -50)), 0.45);
        assert_eq!(at(glue::AQUIFER_FLOODEDNESS, block(far, -50)), VANILLA[1]);
        // Выше −41 затопленность ванильная всюду.
        assert_eq!(at(glue::AQUIFER_FLOODEDNESS, block(near, -40)), VANILLA[1]);
        // Исключение снято только в глубине у стыка.
        assert_eq!(at(glue::AQUIFER_EXCLUSION, block(near, -60)), -1.0);
        assert_eq!(at(glue::AQUIFER_EXCLUSION, block(near, -40)), VANILLA[4]);
        assert_eq!(at(glue::AQUIFER_EXCLUSION, block(far, -60)), VANILLA[4]);
        // Разброс спрашивают в (x div 16, y div 40, z div 16): у стыка в
        // глубинном слое он даёт уровень −60 + floor(7,5 / 3)·3 = −54.
        let spread = at(glue::AQUIFER_SPREAD, [near[0] * 4 + 1, -2, near[1] * 4 + 1]);
        assert_eq!(spread, 0.75);
        let level = -60 + ((f64::from(spread * 10.0) / 3.0).floor() as i32) * 3;
        assert_eq!(level, -54);
        // Поле лавы в глубинном слое у стыка — всегда лава.
        assert_eq!(at(glue::AQUIFER_LAVA, [near[0], -2, near[1]]), 0.5);
        // Поверхность не меняется.
        assert_eq!(at(glue::AQUIFER_SURFACE, block(near, 0)), VANILLA[5]);

        // Объёмный запрос поверхности (так начинает каждый аквифер).
        let volume = DensityVolume::new([2, 1, 2], [0, 0, 0], [4, 1, 4]).unwrap();
        let mut buffer = [0.0_f32; 4];
        glue::deep_lava_reset();
        assert!(!glue::deep_lava_by_aquifer());
        wrapped[glue::AQUIFER_SURFACE]
            .sample_volume(&mut SamplerContext::default(), &mut buffer, volume)
            .unwrap();
        assert_eq!(buffer, [VANILLA[5]; 4]);
        assert!(glue::deep_lava_by_aquifer());
        glue::deep_lava_reset();
    }

    #[test]
    fn barrier_walls_only_the_border_of_lava_and_water_cells() {
        let generator = generator();
        let mut walls = 0;
        let mut plain = 0;
        for cx in -12_i64..12 {
            for cz in -12_i64..12 {
                // Середина ячейки по всем осям: стенки нет никогда.
                let centre = generator.aquifer_barrier(cx * 64 + 32, -20, cz * 64 + 32, 0.125);
                assert_eq!(centre, 0.125);
                // У западной грани: стенка тогда и только тогда, когда
                // соседи по X различаются.
                let own = generator.lava_aquifer(cx, -1, cz) != 0.0;
                let west = generator.lava_aquifer(cx - 1, -1, cz) != 0.0;
                let value = generator.aquifer_barrier(cx * 64 + 3, -20, cz * 64 + 32, 0.125);
                if own != west {
                    assert_eq!(value, 4.0);
                    walls += 1;
                } else {
                    assert_eq!(value, 0.125);
                    plain += 1;
                }
            }
        }
        assert!(walls > 0 && plain > 0, "стенок {walls}, без стенки {plain}");
        // Выше нуля лавы аквиферов нет, барьер ванильный.
        assert_eq!(generator.aquifer_barrier(3, 10, 3, 0.125), 0.125);
    }

    mod reset_hook {
        use crate::horizon_core;
        include!("../../../mod/glue/aquifer_reset_hook.rs.in");
        pub fn call() {
            litospherity_aquifer_reset();
        }
    }
    mod is_lava_hook {
        use crate::horizon_core;
        include!("../../../mod/glue/is_lava_hook.rs.in");
        pub fn call() -> core::ops::ControlFlow<bool> {
            litospherity_is_lava()
        }
    }

    #[test]
    fn is_lava_is_cancelled_only_while_the_thread_works_on_our_aquifer() {
        reset_hook::call();
        assert!(matches!(
            is_lava_hook::call(),
            core::ops::ControlFlow::Continue(())
        ));
        let wrapped = glue::aquifer_samplers(&fields(our_lava())).unwrap();
        // Сборка обёрток сама поток не помечает: помечает первый запрос.
        assert!(matches!(
            is_lava_hook::call(),
            core::ops::ControlFlow::Continue(())
        ));
        wrapped[glue::AQUIFER_SURFACE]
            .sample_value(&mut SamplerContext::default(), [0, 0, 0])
            .unwrap();
        assert!(matches!(
            is_lava_hook::call(),
            core::ops::ControlFlow::Break(false)
        ));
        // Следующий аквифер (любого мира) начинает с чистой пометки.
        reset_hook::call();
        assert!(matches!(
            is_lava_hook::call(),
            core::ops::ControlFlow::Continue(())
        ));
        let _ = horizon_core::litospherity_glue::deep_lava_by_aquifer();
    }

    /// Подставные типы material_condition/library.rs для помощника сборки.
    mod compile_hook {
        use crate::density::DensitySampler;
        use crate::horizon_core;
        use std::sync::Arc;

        #[derive(Debug)]
        pub struct CompileError;
        #[derive(Debug)]
        pub struct DecodeError;
        impl From<CompileError> for DecodeError {
            fn from(_: CompileError) -> Self {
                Self
            }
        }
        pub struct DensityCompiler {
            pub fail: bool,
        }
        pub struct MaterialRuleLibrary {
            pub compiler: DensityCompiler,
        }
        pub struct AquiferSamplers {
            samplers: [Arc<dyn DensitySampler>; 6],
        }
        impl AquiferSamplers {
            pub fn new(samplers: [Arc<dyn DensitySampler>; 6]) -> Self {
                Self { samplers }
            }
            pub fn get(
                &self,
                field: crate::worldgen::aquifer::AquiferField,
            ) -> &Arc<dyn DensitySampler> {
                &self.samplers[field as usize]
            }
        }
        pub struct AquiferConfig {
            pub fields: [Arc<dyn DensitySampler>; 6],
        }
        impl AquiferConfig {
            pub fn compile(
                self,
                compiler: &mut DensityCompiler,
            ) -> Result<AquiferSamplers, CompileError> {
                if compiler.fail {
                    return Err(CompileError);
                }
                Ok(AquiferSamplers::new(self.fields))
            }
        }
        include!("../../../mod/glue/aquifer_hook.rs.in");
        pub fn call(
            library: &mut MaterialRuleLibrary,
            config: AquiferConfig,
        ) -> Result<AquiferSamplers, DecodeError> {
            match litospherity_aquifer(library, config) {
                core::ops::ControlFlow::Break(result) => result,
                core::ops::ControlFlow::Continue(()) => panic!("помощник всегда отвечает сам"),
            }
        }
    }

    #[test]
    fn compile_hook_wraps_our_world_and_keeps_vanilla_and_errors() {
        use crate::worldgen::aquifer::AquiferField;
        use compile_hook::{AquiferConfig, DensityCompiler, MaterialRuleLibrary, call};
        let mut library = MaterialRuleLibrary {
            compiler: DensityCompiler { fail: false },
        };
        let generator = generator();
        let (_, far) = cells(&generator);
        let probe = [(far[0] * 64 + 20) as i32, -60, (far[1] * 64 + 20) as i32];
        let flood = |samplers: &compile_hook::AquiferSamplers| {
            samplers
                .get(AquiferField::Floodedness)
                .sample_value(&mut SamplerContext::default(), probe)
                .unwrap()
        };
        // Ванильный мир: поля те же, затопленность ванильная.
        let vanilla = call(
            &mut library,
            AquiferConfig {
                fields: fields(Arc::new(Constant(0.0))),
            },
        )
        .unwrap();
        assert_eq!(flood(&vanilla), VANILLA[1]);
        // Наш мир: вдали от стыка ниже −54 сухо.
        let ours = call(
            &mut library,
            AquiferConfig {
                fields: fields(our_lava()),
            },
        )
        .unwrap();
        assert_eq!(flood(&ours), -1.0);
        // Ошибка сборки проходит наружу как ошибка, а не теряется.
        library.compiler.fail = true;
        assert!(
            call(
                &mut library,
                AquiferConfig {
                    fields: fields(our_lava()),
                },
            )
            .is_err()
        );
        glue::deep_lava_reset();
    }
}

/// Путь `crate::worldgen::aquifer::AquiferField` из помощника сборки.
mod worldgen {
    pub mod aquifer {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(usize)]
        pub enum AquiferField {
            Barrier,
            Floodedness,
            Spread,
            Lava,
            Exclusion,
            SurfaceLevel,
        }
        impl AquiferField {
            pub const ALL: [Self; 6] = [
                Self::Barrier,
                Self::Floodedness,
                Self::Spread,
                Self::Lava,
                Self::Exclusion,
                Self::SurfaceLevel,
            ];
        }
    }
}
