// Litospherity: the module crate::game::litospherity. Horizon-free, so it also builds with cargo test.
pub const ID: &str = "litospherity";
pub const NAME: &str = "Litospherity";
pub const VERSION: &str = "0.1.0";
pub const DESCRIPTION: &str = "Tectonic plates and climate world generation (work in progress)";

/// Text for /litospherity.
pub fn status() -> String {
    format!(
        "{NAME} {VERSION} is loaded. Worldgen hook installed. Enable exactly one Litospherity datapack when creating a new world."
    )
}
