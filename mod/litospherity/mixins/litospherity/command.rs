// Хук native_command (HEAD): обрабатывает команды мода (/litospherity ...).
// По образцу distant_horizons/mixins/distant_horizons/command.rs.

pub(super) fn command(
    game: &mut crate::game::Game<'_>,
    command: &str,
    ran: &mut bool,
) {
    let _ = (game, command, ran);
    // TODO: обработать команду /litospherity.
}
