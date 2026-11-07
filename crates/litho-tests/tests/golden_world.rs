//! Full output freeze after the intentional 0.3.0 boundary/climate policy fixes.
//! The pre-cache 0.2.0 baseline remains in golden/v0/world.txt for history.
#[path = "../../../tools/world_snapshot.rs"]
mod world_snapshot;

#[test]
fn world_v0_matches_0_3_0_generation_bits() {
    let actual = world_snapshot::snapshot();
    let expected = include_str!("../../../golden/v0/world-0.3.0.txt");
    assert_eq!(actual.lines().count(), expected.lines().count());
    for (line, (actual, expected)) in actual.lines().zip(expected.lines()).enumerate() {
        assert_eq!(actual, expected, "world snapshot line {}", line + 1);
    }
}
