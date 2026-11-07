// Read-only diagnostics use explicit inputs; they do not change a world.
fn litospherity_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    let words: Vec<_> = command.split_whitespace().collect();
    if *ran || words.first().copied() != Some("litospherity") {
        return;
    }
    *ran = true;
    let message = if words.len() == 1 {
        crate::game::litospherity::status()
    } else if words.len() == 6 && words[1] == "probe" {
        match (
            words[2].parse::<i64>(),
            words[4].parse::<f64>(),
            words[5].parse::<f64>(),
        ) {
            (Ok(seed), Ok(x), Ok(z)) => {
                horizon_core::litospherity_glue::diagnose(seed, words[3], x, z)
            }
            _ => "Invalid seed or coordinates".into(),
        }
    } else {
        "Usage: /litospherity probe <seed> <gameplay|scientific> <x> <z>".into()
    };
    log::info!("{message}");
    game.native_feedback(horizon_client::text::Text::literal(message));
}
