// /litospherity: says the mod is loaded.
fn litospherity_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    if *ran || command.trim() != "litospherity" {
        return;
    }
    *ran = true;
    game.native_feedback(horizon_client::text::Text::literal(crate::game::litospherity::status()));
}
