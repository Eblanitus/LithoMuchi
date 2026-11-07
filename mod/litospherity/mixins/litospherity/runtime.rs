// Litospherity runtime: наша генерация мира (не зависит от Horizon).
//
// Может использовать crates/litho-core как библиотеку.
// Все функции здесь — Horizon-free, тестируются отдельно через cargo test.

/// Идентификатор мода.
pub const ID: &str = "litospherity";
pub const NAME: &str = "Litospherity";
pub const VERSION: &str = "0.1.0";
pub const DESCRIPTION: &str = "Litospherity planet generator ported to Horizon Minecraft 26.3";

/// Точка входа для инициализации мода (вызывается при загрузке).
pub fn init() {
    // TODO: регистрация шумовых источников, загрузка конфига.
}

/// Генерирует высоту рельефа для точки (x, z) в блоках.
/// Возвращает высоту поверхности (y) и биом.
pub fn surface_height(x: i32, z: i32, seed: u64) -> (i32, u16) {
    // TODO: реальный вызов litho-core::Generator::crust_sample или аналогичного.
    //       Сейчас заглушка — плоский мир на уровне моря.
    (64, 0) // (y, biome_id)
}

/// Генерирует плотность блока для колонки (x, z) на высоте y.
pub fn block_density(x: i32, y: i32, z: i32, seed: u64) -> f32 {
    // TODO: реальный вызов litho-core через bridge.
    //       Сейчас заглушка — всё воздух.
    0.0
}
