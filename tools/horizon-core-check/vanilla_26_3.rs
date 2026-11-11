// Executed in the staged real horizon-world, not as part of this workspace.
// Inputs: generated packs and mcmeta's pinned 26.3-data-json checkout.
// К2 / шаг 2: depth at y=80 is 0.375 + offset and the encoded surface is
// y = 128·(1 + offset). factor/jaggedness stay vanilla for test №2.
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use horizon_core::density::graph::DensityCompiler;
use horizon_core::density::{DensitySampler, SamplerContext};
use horizon_core::litospherity_glue::field_for;
use horizon_world::worldgen::density::DensityRegistry;
use horizon_world::worldgen::overrides::{PackFile, WorldgenOverrides};

fn add_pack(overrides: &mut WorldgenOverrides, root: &Path) {
    fn visit(o: &mut WorldgenOverrides, root: &Path, dir: &Path) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(o, root, &path);
            } else if path.extension().is_some_and(|v| v == "json") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('\\', "/");
                assert_eq!(
                    o.add_file("test", &rel, || std::fs::read_to_string(&path).ok()),
                    PackFile::Read,
                    "{rel}"
                );
            }
        }
    }
    visit(overrides, root, &root.join("data"));
}

fn samplers(pack: &str) -> Vec<Arc<dyn DensitySampler>> {
    let packs = std::env::var("LITHO_PACKS").unwrap();
    let source = std::env::var("LITHO_26_3_DATA").unwrap();
    let data = Path::new(&source).join("data/minecraft/worldgen/density_function/overworld");
    let mut densities = BTreeMap::new();
    for field in ["offset", "factor", "jaggedness", "ridges_folded", "depth"] {
        densities.insert(
            format!("minecraft:overworld/{field}"),
            std::fs::read_to_string(data.join(format!("{field}.json"))).unwrap(),
        );
    }
    let mut overrides = WorldgenOverrides::default();
    add_pack(&mut overrides, &Path::new(&packs).join(pack));
    densities.extend(
        overrides
            .entries("densities")
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
    );
    let noises = overrides
        .entries("noises")
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let registry = DensityRegistry::from_json(&noises, &densities).unwrap();
    let names = [
        "continents",
        "erosion",
        "ridges",
        "ridges_folded",
        "offset",
        "factor",
        "jaggedness",
        "depth",
    ];
    let ids: Vec<_> = names
        .into_iter()
        .map(|name| {
            registry
                .reference(&format!("minecraft:overworld/{name}"))
                .unwrap()
        })
        .collect();
    let mut compiler = DensityCompiler::new_seeded(registry.into_graph(), 1, false);
    ids.into_iter()
        .map(|id| compiler.get_sampler(id).unwrap())
        .collect()
}

fn sample(fields: &[Arc<dyn DensitySampler>], p: [i32; 3]) -> Vec<f32> {
    fields
        .iter()
        .map(|f| f.sample_value(&mut SamplerContext::default(), p).unwrap())
        .collect()
}

#[test]
fn v1_dev_pack_reaches_real_26_3_depth_graph() {
    let pack = "litospherity_v1-dev_gameplay";
    let root = Path::new(&std::env::var("LITHO_PACKS").unwrap()).join(pack);
    let df = root.join("data/minecraft/worldgen/density_function/overworld");
    let noise = root.join("data/litospherity/worldgen/noise");
    assert!((df.join("ridges_folded.json")).is_file());
    assert!((df.join("offset.json")).is_file());
    for field in ["factor", "jaggedness"] {
        assert!(
            !df.join(format!("{field}.json")).exists(),
            "пак не должен переопределять {field} до теста №2"
        );
        assert!(
            noise.rglob(&format!("{field}.json")).next().is_none(),
            "пак не должен объявлять шум {field} до теста №2"
        );
    }

    let fields = samplers(pack);
    let mut checked_surface = false;
    for x in [-512, -256, 0, 256, 512] {
        for z in [-512, -256, 0, 256, 512] {
            let a = sample(&fields, [x, 80, z]);
            assert!(a.iter().all(|v| v.is_finite()), "нечисловое поле в ({x}, {z})");
            // Настоящий градиент 26.3 на y=80 равен 0,375.
            assert!(
                (a[7] - (0.375 + a[4])).abs() < 1e-5,
                "depth({x},80,{z}) = {} при offset {}",
                a[7],
                a[4]
            );
            // offset должен прийти именно из litho-core, а не из ванили.
            let expected = field_for(
                1,
                "litospherity:v1-dev/gameplay/offset",
                f64::from(x),
                f64::from(z),
            )
            .unwrap();
            assert!(
                (a[4] - expected).abs() < 1e-6,
                "offset({x},{z}) = {} != ядра {}",
                a[4],
                expected
            );
            assert!((-2.0..=2.0).contains(&a[4]));
            // factor/jaggedness остаются ванильными: только разумные пределы.
            assert!((0.0..=10.0).contains(&a[5]));
            assert!((0.0..=1.0).contains(&a[6]));

            // Поверхность: depth(y) = (1 − y/128) + offset; при
            // offset = y/128 − 1 она равна нулю на заданной высоте.
            let surface_y = (128.0 * (1.0 + f64::from(a[4]))).round() as i32;
            assert!((-64..=319).contains(&surface_y), "surface_y={surface_y}");
            let at_surface = sample(&fields, [x, surface_y, z]);
            let gradient = 1.0 - surface_y as f32 / 128.0;
            assert!(
                (at_surface[7] - (gradient + at_surface[4])).abs() < 1e-5,
                "depth на поверхности {} != {}",
                at_surface[7],
                gradient + at_surface[4]
            );
            assert!(
                at_surface[7].abs() < 0.01,
                "поверхность ({x},{surface_y},{z}) не на нулевой глубине: {}",
                at_surface[7]
            );
            checked_surface = true;
        }
    }
    assert!(checked_surface);
}
