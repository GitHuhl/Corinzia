use corinzia_byz::*;

#[test]
fn the_demo_adds_up() {
    let (sung, problems) = demo().sing();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(sung.first().unwrap().note.label(), "Pa");
    assert_eq!(sung.last().unwrap().note.label(), "Pa");
}
