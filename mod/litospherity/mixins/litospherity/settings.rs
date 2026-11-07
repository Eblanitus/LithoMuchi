// Хук native_settings (HEAD): добавляет кнопки настроек мода.
// По образцу distant_horizons/mixins/distant_horizons/settings.rs.

pub(super) fn settings(
    game: &mut crate::game::Game<'_>,
    mod_id: &str,
    buttons: &mut Vec<String>,
) {
    let _ = (game, mod_id, buttons);
    // TODO: добавить кнопки настроек Litospherity.
}
