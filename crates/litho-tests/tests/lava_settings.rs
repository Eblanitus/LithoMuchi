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
    let _ = litospherity_lava_settings(None, &mut settings, data, dimension);
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
        assert_eq!(lava["max_exclusive"], 0);
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

#[test]
fn scientific_changes_only_terrain_slides_not_build_height_or_cave_terms() {
    let original = json!({
        "noise": {"min_y": -64, "height": 384}, "sea_level": 63,
        "noise_router": {"final_density": {
            "type": "add", "left": {"type": "gradient", "axis": "y",
                "from_coordinate":240,"to_coordinate":256,"from_value":1,"to_value":0},
            "right": {"type":"add", "left":"minecraft:overworld/caves/noodle",
                "right":{"type":"gradient","axis":"y","from_coordinate":-64,
                    "to_coordinate":-40,"from_value":0,"to_value":1}}
        }}
    });
    let gameplay = patch(
        original.clone(),
        &data(&["gameplay"]),
        "minecraft:overworld",
    );
    assert_eq!(gameplay, original);
    let mut scientific_data = data(&["scientific"]);
    // Legacy scientific packs do not opt into new terrain handling.
    assert_eq!(
        patch(original.clone(), &scientific_data, "minecraft:overworld"),
        original
    );
    Arc::get_mut(&mut scientific_data)
        .unwrap()
        .densities
        .insert(
            "litospherity:v0/scientific/height_active".into(),
            "0.0".into(),
        );
    let scientific = patch(original.clone(), &scientific_data, "minecraft:overworld");
    assert_eq!(scientific["noise"], original["noise"]);
    assert_eq!(scientific["sea_level"], original["sea_level"]);
    assert_eq!(
        scientific["noise_router"]["final_density"]["left"]["from_coordinate"],
        288
    );
    assert_eq!(
        scientific["noise_router"]["final_density"]["left"]["to_coordinate"],
        312
    );
    assert_eq!(
        scientific["noise_router"]["final_density"]["right"]["right"]["to_coordinate"],
        -56
    );
    assert_eq!(
        scientific["noise_router"]["final_density"]["right"]["left"],
        "minecraft:overworld/caves/noodle"
    );
    for dim in ["minecraft:the_nether", "minecraft:the_end"] {
        assert_eq!(
            patch(original.clone(), &data(&["scientific"]), dim),
            original
        );
    }
    let mut referenced = original.clone();
    referenced["noise_router"]["final_density"] = "minecraft:test".into();
    let mut registry = scientific_data;
    Arc::get_mut(&mut registry).unwrap().densities.insert(
        "minecraft:test".into(),
        original["noise_router"]["final_density"].to_string(),
    );
    assert_eq!(
        patch(referenced, &registry, "minecraft:overworld"),
        scientific
    );
}
