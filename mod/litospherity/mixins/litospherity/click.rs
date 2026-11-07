// Хук native_settings_click (HEAD): обрабатывает клик по кнопке настроек.
// По образцу distant_horizons/mixins/distant_horizons/click.rs.

pub(super) fn click(
    game: &mut crate::game::Game<'_>,
    mod_id: &str,
    button: usize,
) {
    let _ = (game, mod_id, button);
    // TODO: обработать клик по кнопке настроек Litospherity.
}
