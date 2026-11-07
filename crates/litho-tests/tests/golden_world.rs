//! Full output freeze, captured from the pre-column-cache implementation.
#[path = "../../../tools/world_snapshot.rs"]
mod world_snapshot;

#[test]
fn world_v0_matches_pre_optimization_bits() {
    let actual = world_snapshot::snapshot();
    let expected = include_str!("../../../golden/v0/world.txt");
    assert_eq!(actual.lines().count(), expected.lines().count());
    for (line, (actual, expected)) in actual.lines().zip(expected.lines()).enumerate() {
        assert_eq!(actual, expected, "world snapshot line {}", line + 1);
    }
}
