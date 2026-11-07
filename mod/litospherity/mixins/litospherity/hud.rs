// Хук native_hud (HEAD): рисует отладочную информацию мода на HUD.
// По образцу distant_horizons/mixins/distant_horizons/hud.rs.

pub(super) fn hud(
    game: &mut crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    let _ = (game, batch, corners);
    // TODO: нарисовать отладочную информацию Litospherity (секции, чанки, время генерации).
}
