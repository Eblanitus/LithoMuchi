// T142-бис: тестовый маркер «вода, если y ≤ 100 в квадрате 64×64 у (0, 0)».
// Проверка, принимает ли horizon-mixins plan хук на aquifer. Генератор не
// трогаем: это разведка, не фича. Файл вставляется локальной функцией внутрь
// модуля horizon_world::worldgen::aquifer (правило 5.5: ровно ОДНА приватная
// fn и ни одного атрибута), видит приватные поля и типы модуля.
// Вода берётся из this.default_fluid (вода ванили в overworld); level = 101
// даёт столб воды строго ниже y = 101.
fn litospherity_aquifer_marker(
    this: &GlobalFluidPicker,
    position: [i32; 3],
) -> core::ops::ControlFlow<FluidStatus> {
    if position[1] <= 100
        && position[0] >= 0
        && position[0] < 64
        && position[2] >= 0
        && position[2] < 64
    {
        return core::ops::ControlFlow::Break(FluidStatus {
            level: 101,
            state: this.default_fluid,
        });
    }
    core::ops::ControlFlow::Continue(())
}
