use ecliptishock_core::config::Config;
use ecliptishock_core::events::parse_line;
use ecliptishock_core::mapper::Mapper;

/// Replays real damage/death lines captured from an Ecliptica session.
#[test]
fn replays_real_session() {
    let cfg = Config::default().finalize().unwrap();
    let mut m = Mapper::new(cfg);
    let text = std::fs::read_to_string("tests/fixtures/sample.log").unwrap();
    let (mut deaths, mut shocks, mut last) = (0u32, 0u32, 0u64);
    for line in text.lines() {
        let Some(p) = parse_line(line) else { continue };
        if let Some(ts) = p.ts_ms {
            last = ts;
        }
        for s in m.handle(last, &p.event) {
            shocks += 1;
            if s.tier == "death" {
                deaths += 1;
            }
        }
    }
    for s in m.advance(last + 3_600_000) {
        shocks += 1;
        if s.tier == "death" {
            deaths += 1;
        }
    }
    assert_eq!(deaths, 6, "the session contains six distinct deaths");
    assert!(shocks > deaths, "damage should also have produced shocks");
}
