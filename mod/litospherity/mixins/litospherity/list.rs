// Хук native_mods (HEAD): регистрирует наш мод в списке модов.
// По образцу distant_horizons/mixins/distant_horizons/list.rs.

pub(super) fn list(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    // TODO: добавить litospherity в список доступных модов.
    let _ = mods;
}
