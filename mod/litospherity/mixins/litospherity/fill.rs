// Хук native_fill (HEAD): точка входа для генерации плотности чанка.
// По коду Horizon 66f3981 (native-hooks.json).
//
// TODO: проверить точную сигнатуру native_fill в коде Horizon версии 66f3981.
//       Привязки (bindings) в horizon-mixin.toml могут потребовать корректировки.

pub(super) fn fill(game: &mut crate::game::Game<'_>) {
    let _ = game;
    // TODO: вызвать LitospherityGenerator::generate_chunk_density через bridge.
    //       Передать координаты чанка, получить объём плотности, записать в игру.
}
