use lorvaq::app;
use std::time::Duration;

#[tokio::test]
async fn demo_builds_the_scene_within_seconds() {
    let scenes = app::demo_scenes().unwrap();
    assert_eq!(scenes.len(), 1);
    let ci = scenes[0].clone();
    assert_eq!((ci.id, ci.title), ("ci", "CI"));
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    let snap = ci.snapshot();
    assert!(snap.seq > 0, "events were published");
    assert!(
        snap.places.iter().any(|p| p.kind == "slot"),
        "agent reports created slots"
    );
    assert!(
        snap.places.iter().any(|p| p.id == "exit:registry"),
        "demo exits exist"
    );
}
