//! Compile the shipped settings helper, using the same Value/Arc types.
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc};
struct GeneratorData {
    densities: BTreeMap<String, String>,
}
#[derive(Clone, Copy)]
struct NoiseSettings;
include!("../../../mod/glue/lava_settings.rs.in");

fn patch(mut settings: Value, data: &Arc<GeneratorData>, dimension: &str) -> Value {
    litospherity_lava_settings(None, &mut settings, data, dimension);
    settings
}

fn data(modes: &[&str]) -> Arc<GeneratorData> {
    Arc::new(GeneratorData {
        densities: modes
            .iter()
            .map(|m| (format!("litospherity:v0/{m}/active"), "0.0".into()))
            .collect(),
    })
}

#[test]
fn only_our_overworld_changes_and_original_lava_is_preserved_above_depth_gate() {
    let original = json!({"noise": {"min_y": -64}, "aquifers": {"lava": {"type": "original"}, "barrier": 0.25}});
    for dimension in ["minecraft:the_nether", "minecraft:the_end", "custom:world"] {
        assert_eq!(
            patch(original.clone(), &data(&["gameplay"]), dimension),
            original
        );
    }
    for modes in [vec![], vec!["gameplay", "scientific"]] {
        assert_eq!(
            patch(original.clone(), &data(&modes), "minecraft:overworld"),
            original
        );
    }
    for mode in ["gameplay", "scientific"] {
        let changed = patch(original.clone(), &data(&[mode]), "minecraft:overworld");
        assert_eq!(changed["noise"], original["noise"]);
        assert_eq!(
            changed["aquifers"]["barrier"],
            original["aquifers"]["barrier"]
        );
        let lava = &changed["aquifers"]["lava"];
        assert_eq!(lava["when_out_of_range"], original["aquifers"]["lava"]);
        assert_eq!(
            lava["when_in_range"],
            format!("litospherity:v0/{mode}/lava")
        );
        assert_eq!(lava["input"]["axis"], "y");
        assert_eq!(lava["max_exclusive"], -79);
    }
    let no_aquifer = json!({"noise": {"min_y": 0}});
    assert_eq!(
        patch(
            no_aquifer.clone(),
            &data(&["gameplay"]),
            "minecraft:overworld"
        ),
        no_aquifer
    );
}
