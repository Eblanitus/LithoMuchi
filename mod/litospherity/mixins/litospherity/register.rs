// /litospherity, open to everyone.
fn litospherity_register(commands: &mut Vec<crate::game::native_server::NativeCommand>) {
    commands.push(crate::game::native_server::NativeCommand::op(
        "litospherity",
        0,
    ));
    commands.push(crate::game::native_server::NativeCommand::op(
        "litospherity probe <seed:word> <mode:word> <x:number> <z:number>",
        0,
    ));
}
