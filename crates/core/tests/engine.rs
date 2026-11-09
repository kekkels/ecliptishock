use ecliptishock_core::config::Config;
use ecliptishock_core::engine::{self, Control, StartOpts};
use ecliptishock_core::report::{Outcome, Reporter, UiEvent};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

fn cfg() -> Config {
    toml::from_str::<Config>(
        r#"
dry_run = true

[[targets]]
name = "me"
token = "t1"
shockers = ["a"]
is_me = true

[[targets]]
name = "friend"
token = "t2"
shockers = ["b"]
"#,
    )
    .unwrap()
    .finalize()
    .unwrap()
}

fn append(path: &std::path::Path, lines: &[&str]) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    for l in lines {
        writeln!(f, "2026.11.08 20:00:00 Debug      -  {l}").unwrap();
    }
}

fn deliveries(events: &Mutex<Vec<UiEvent>>) -> Vec<(String, String)> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            UiEvent::Delivery { tier, target, outcome: Outcome::DryRun, .. } => Some((tier.clone(), target.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn solo_then_shared_end_to_end() {
    let dir = std::env::temp_dir().join(format!("eclip-engine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join("output_log_2026-11-08_20-00-00.txt");
    std::fs::write(&log, "2026.11.08 20:00:00 Debug      -  ECLIPTICA - now in lobby\n").unwrap();

    let events = Arc::new(Mutex::new(Vec::<UiEvent>::new()));
    let sink = events.clone();
    let report: Reporter = Arc::new(move |e| sink.lock().unwrap().push(e));
    let (handle, rx) = engine::channel();
    let task = tokio::spawn(engine::run(
        StartOpts { cfg: cfg(), log_dir: Some(dir.clone()), shared_override: None, force_dry_run: false },
        report,
        rx,
    ));
    sleep(Duration::from_millis(300)).await;

    // In a fight, one ordinary hit, solo: only "me".
    append(&log, &["ECLIPTICA - now in stage: Stage_Hallow on phase: 1 as class: Shieldmage"]);
    sleep(Duration::from_millis(300)).await;
    append(&log, &["damage has been taken: 34, from source: attack_Stab"]);
    sleep(Duration::from_millis(700)).await;
    assert_eq!(deliveries(&events), vec![("medium".into(), "me".into())]);

    // Six DoT ticks become ONE shock once they stop.
    for _ in 0..6 {
        append(&log, &["damage has been taken: 4, from source: "]);
        sleep(Duration::from_millis(100)).await;
    }
    sleep(Duration::from_millis(1700)).await;
    assert_eq!(deliveries(&events).len(), 2, "{:?}", deliveries(&events));
    assert_eq!(deliveries(&events)[1], ("light".to_string(), "me".to_string()));

    // Shared fate on: a big hit reaches everyone.
    handle.send(Control::SetShared(true));
    sleep(Duration::from_millis(100)).await;
    append(&log, &["damage has been taken: 60, from source: spinAttack"]);
    sleep(Duration::from_millis(700)).await;
    let d = deliveries(&events);
    assert_eq!(d[2..], [("heavy".to_string(), "me".to_string()), ("heavy".to_string(), "friend".to_string())]);

    // Death printed three times -> one shock; shared fate off again -> only "me".
    handle.send(Control::SetShared(false));
    append(&log, &["Local controller dead, switching off.", "Local controller dead, switching off.", "Local controller dead, switching off."]);
    sleep(Duration::from_millis(400)).await;
    let d = deliveries(&events);
    assert_eq!(d.len(), 5, "{d:?}");
    assert_eq!(d[4], ("death".to_string(), "me".to_string()));

    // Pausing drops instead of queueing.
    handle.send(Control::SetPaused(true));
    handle.send(Control::Test("light".into()));
    sleep(Duration::from_millis(200)).await;
    assert!(events.lock().unwrap().iter().any(|e| matches!(e, UiEvent::Delivery { outcome: Outcome::Paused, .. })));

    handle.send(Control::Stop);
    task.await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn reports_a_missing_log_folder() {
    let events = Arc::new(Mutex::new(Vec::<UiEvent>::new()));
    let sink = events.clone();
    let report: Reporter = Arc::new(move |e| sink.lock().unwrap().push(e));
    let (handle, rx) = engine::channel();
    let task = tokio::spawn(engine::run(
        StartOpts {
            cfg: cfg(),
            log_dir: Some("/definitely/not/a/real/folder".into()),
            shared_override: None,
            force_dry_run: false,
        },
        report,
        rx,
    ));
    sleep(Duration::from_millis(300)).await;
    handle.send(Control::Stop);
    task.await.unwrap();
    let ev = events.lock().unwrap();
    assert!(ev.iter().any(|e| matches!(e, UiEvent::Status(s) if s.error.as_deref().map_or(false, |m| m.contains("does not exist")) && !s.attached)));
}
