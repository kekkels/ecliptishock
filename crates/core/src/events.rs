use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Damage { amount: u32, source: String },
    Death,
    /// A stage or boss line: a run is in progress.
    Arm,
    /// Lobby or intermission: nothing should fire.
    Disarm,
}

#[derive(Debug, Clone)]
pub struct Parsed {
    /// Log-line timestamp in ms (second resolution). Only used for replays.
    pub ts_ms: Option<u64>,
    pub event: Event,
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("regex"))
}

pub fn parse_line(line: &str) -> Option<Parsed> {
    static DMG: OnceLock<Regex> = OnceLock::new();
    static LOBBY: OnceLock<Regex> = OnceLock::new();

    let event = if let Some(c) = re(&DMG, r"damage has been taken: (\d+), from source:\s*(.*?)\s*$").captures(line) {
        Event::Damage { amount: c[1].parse().ok()?, source: c[2].to_string() }
    } else if line.contains("Local controller dead") {
        Event::Death
    } else if re(&LOBBY, r"ECLIPTICA - now in (lobby|intermission)").is_match(line) {
        Event::Disarm
    } else if line.contains("ECLIPTICA - now in stage") || line.contains("ECLIPTICA - now fighting boss") {
        Event::Arm
    } else {
        return None;
    };
    Some(Parsed { ts_ms: parse_ts(line), event })
}

/// "2026.11.06 09:45:17 ..." -> ms since the Unix epoch (treated as UTC; only differences matter).
pub fn parse_ts(line: &str) -> Option<u64> {
    static TS: OnceLock<Regex> = OnceLock::new();
    let c = re(&TS, r"^(\d{4})\.(\d{2})\.(\d{2}) (\d{2}):(\d{2}):(\d{2})").captures(line)?;
    let n = |i: usize| c[i].parse::<i64>().ok();
    let days = days_from_civil(n(1)?, n(2)?, n(3)?);
    let secs = days * 86400 + n(4)? * 3600 + n(5)? * 60 + n(6)?;
    u64::try_from(secs * 1000).ok()
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_damage_death_and_state() {
        let d = parse_line("2026.11.06 09:45:10 Debug      -  damage has been taken: 82, from source: attack_GorillaSlam").unwrap();
        assert_eq!(d.event, Event::Damage { amount: 82, source: "attack_GorillaSlam".into() });
        let t = parse_line("2026.11.06 09:45:11 Debug      -  damage has been taken: 3, from source: ").unwrap();
        assert_eq!(t.event, Event::Damage { amount: 3, source: String::new() });
        assert_eq!(parse_line("2026.11.06 09:45:17 Debug      -  Local controller dead, switching off.").unwrap().event, Event::Death);
        assert_eq!(parse_line("x ECLIPTICA - now in intermission").unwrap().event, Event::Disarm);
        assert_eq!(parse_line("x ECLIPTICA - now in lobby").unwrap().event, Event::Disarm);
        assert_eq!(parse_line("x ECLIPTICA - now in stage: Stage_Hallow on phase: 1 as class: Spellsword").unwrap().event, Event::Arm);
        assert_eq!(parse_line("x ECLIPTICA - now fighting boss: Bravera(Clone) on phase: 1").unwrap().event, Event::Arm);
        assert!(parse_line("something unrelated").is_none());
    }

    #[test]
    fn timestamps_are_ordered() {
        let a = parse_ts("2026.11.06 09:45:17 x").unwrap();
        let b = parse_ts("2026.11.06 09:45:18 x").unwrap();
        let c = parse_ts("2026.11.07 00:00:00 x").unwrap();
        assert_eq!(b - a, 1000);
        assert!(c > b);
    }
}
