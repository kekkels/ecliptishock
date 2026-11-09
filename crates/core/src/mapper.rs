//! Pure state machine: log events in, shocks out. No clock, no I/O, so it can be replayed.
use crate::config::{Config, Kind};
use crate::events::Event;

#[derive(Debug, Clone, PartialEq)]
pub struct Shock {
    pub tier: String,
    pub kind: Kind,
    pub intensity: u8,
    pub duration_ms: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Copy)]
struct Pool {
    total: u32,
    hits: u32,
    first: u64,
    last: u64,
}

impl Pool {
    fn new(now: u64, amt: u32) -> Self {
        Pool { total: amt, hits: 1, first: now, last: now }
    }
    fn add(&mut self, now: u64, amt: u32) {
        self.total = self.total.saturating_add(amt);
        self.hits += 1;
        self.last = now;
    }
}

pub struct Mapper {
    cfg: Config,
    armed: bool,
    last_death: Option<u64>,
    suppress_until: u64,
    /// Real hits merging inside `burst_ms`.
    burst: Option<Pool>,
    /// DoT ticks pooled until things go quiet.
    dot: Option<Pool>,
    last_shock: Option<(u64, usize)>,
}

impl Mapper {
    pub fn new(cfg: Config) -> Self {
        let armed = !cfg.log.only_in_fight;
        Mapper { cfg, armed, last_death: None, suppress_until: 0, burst: None, dot: None, last_shock: None }
    }

    /// Rebuild fight state from old log lines at startup without firing anything.
    pub fn prime(&mut self, ev: &Event) {
        if self.cfg.log.only_in_fight {
            match ev {
                Event::Arm => self.armed = true,
                Event::Disarm => self.armed = false,
                _ => {}
            }
        }
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// Forget all fight state, e.g. when VRChat starts a fresh log.
    pub fn reset(&mut self) {
        self.armed = !self.cfg.log.only_in_fight;
        self.burst = None;
        self.dot = None;
        self.last_death = None;
        self.suppress_until = 0;
        self.last_shock = None;
    }

    /// Flush any pool whose timer has expired.
    pub fn advance(&mut self, now: u64) -> Vec<Shock> {
        let mut out = Vec::new();
        if let Some(p) = self.burst {
            if now >= p.first + self.cfg.damage.burst_ms {
                self.burst = None;
                let reason = format!("{} hit(s), {} dmg", p.hits, p.total);
                out.extend(self.emit(now, p.total, reason));
            }
        }
        if let Some(p) = self.dot {
            let quiet = now >= p.last + self.cfg.dot.quiet_ms;
            let held = now >= p.first + self.cfg.dot.max_hold_ms;
            if quiet || held {
                self.dot = None;
                let total = (p.total as f64 * self.cfg.dot.scale).round() as u32;
                let reason = format!("dot: {} ticks, {} raw dmg", p.hits, p.total);
                out.extend(self.emit(now, total, reason));
            }
        }
        out
    }

    pub fn handle(&mut self, now: u64, ev: &Event) -> Vec<Shock> {
        let mut out = self.advance(now);
        match ev {
            Event::Arm => {
                if self.cfg.log.only_in_fight {
                    self.armed = true;
                }
            }
            Event::Disarm => {
                if self.cfg.log.only_in_fight {
                    self.armed = false;
                    self.burst = None;
                    self.dot = None;
                }
            }
            Event::Death => {
                if !self.armed {
                    return out;
                }
                if let Some(t) = self.last_death {
                    if now.saturating_sub(t) < self.cfg.death.debounce_ms {
                        return out;
                    }
                }
                self.last_death = Some(now);
                self.suppress_until = now + self.cfg.death.suppress_ms;
                self.burst = None;
                self.dot = None;
                out.push(self.death_shock());
            }
            Event::Damage { amount, .. } => {
                if !self.armed || now < self.suppress_until {
                    return out;
                }
                if *amount <= self.cfg.damage.tick_max {
                    if !self.cfg.dot.enabled {
                        return out;
                    }
                    if let Some(p) = self.dot.as_mut() {
                        p.add(now, *amount);
                    } else {
                        self.dot = Some(Pool::new(now, *amount));
                    }
                } else if let Some(p) = self.burst.as_mut() {
                    p.add(now, *amount);
                } else {
                    self.burst = Some(Pool::new(now, *amount));
                }
            }
        }
        out
    }

    /// Build a shock for a named tier (or "death"), for the `test` command.
    pub fn tier_shock(&self, name: &str) -> Option<Shock> {
        if name == "death" {
            return Some(self.death_shock());
        }
        let t = self.cfg.tiers.iter().find(|t| t.name == name)?;
        Some(self.clamp(Shock {
            tier: t.name.clone(),
            kind: t.kind,
            intensity: t.intensity,
            duration_ms: t.duration_ms,
            reason: "test".into(),
        }))
    }

    fn death_shock(&self) -> Shock {
        let d = &self.cfg.death;
        self.clamp(Shock {
            tier: "death".into(),
            kind: d.kind,
            intensity: d.intensity,
            duration_ms: d.duration_ms,
            reason: "death".into(),
        })
    }

    fn clamp(&self, mut s: Shock) -> Shock {
        s.intensity = s.intensity.min(self.cfg.limits.max_intensity).min(100);
        s.duration_ms = s.duration_ms.min(self.cfg.limits.max_duration_ms);
        s
    }

    fn emit(&mut self, now: u64, total: u32, reason: String) -> Option<Shock> {
        let rank = self.cfg.tiers.iter().rposition(|t| t.min_damage <= total)?;
        if let Some((t, r)) = self.last_shock {
            // Inside the cooldown only a higher tier gets through.
            if now.saturating_sub(t) < self.cfg.damage.cooldown_ms && rank <= r {
                return None;
            }
        }
        self.last_shock = Some((now, rank));
        let t = &self.cfg.tiers[rank];
        Some(self.clamp(Shock {
            tier: t.name.clone(),
            kind: t.kind,
            intensity: t.intensity,
            duration_ms: t.duration_ms,
            reason,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Config {
        toml::from_str::<Config>("").unwrap().finalize().unwrap()
    }
    fn dmg(a: u32) -> Event {
        Event::Damage { amount: a, source: "x".into() }
    }
    fn armed(c: Config) -> Mapper {
        let mut m = Mapper::new(c);
        m.handle(0, &Event::Arm);
        m
    }

    #[test]
    fn simultaneous_hits_merge_into_one_shock() {
        let mut m = armed(cfg());
        for i in 0..4 {
            assert!(m.handle(i * 10, &dmg(15)).is_empty());
        }
        let out = m.advance(300);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tier, "heavy"); // 4 x 15 = 60 total
    }

    #[test]
    fn dot_ticks_accumulate_into_a_single_shock() {
        let mut c = cfg();
        c.dot.max_hold_ms = 60_000;
        c.dot.scale = 1.0;
        let mut m = armed(c);
        for i in 0..10u64 {
            assert!(m.handle(1000 + i * 500, &dmg(5)).is_empty(), "no shock while the DoT is running");
        }
        assert!(m.advance(5500 + 1000).is_empty(), "still inside the quiet window");
        let out = m.advance(5500 + 1300);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].tier, "heavy"); // 10 ticks x 5 = 50
        assert!(m.advance(60_000).is_empty());
    }

    #[test]
    fn endless_dot_still_flushes_periodically() {
        let mut m = armed(cfg()); // max_hold 4000, scale 0.5
        let mut fired = 0;
        for i in 0..40u64 {
            fired += m.handle(i * 300, &dmg(7)).len(); // ticks never go quiet
        }
        assert!(fired >= 2, "got {fired}");
    }

    #[test]
    fn lone_tick_does_nothing() {
        let mut m = armed(cfg());
        m.handle(0, &dmg(3));
        assert!(m.advance(10_000).is_empty());
    }

    #[test]
    fn death_is_debounced_and_suppresses_damage() {
        let mut m = armed(cfg());
        assert_eq!(m.handle(1000, &Event::Death).len(), 1);
        assert!(m.handle(1000, &Event::Death).is_empty());
        assert!(m.handle(1400, &Event::Death).is_empty());
        m.handle(2000, &dmg(60));
        assert!(m.advance(3000).is_empty());
    }

    #[test]
    fn nothing_fires_outside_a_fight() {
        let mut m = Mapper::new(cfg());
        m.handle(0, &dmg(60));
        assert!(m.advance(1000).is_empty());
        assert!(m.handle(1000, &Event::Death).is_empty());
        m.handle(2000, &Event::Arm);
        m.handle(3000, &Event::Disarm);
        m.handle(3100, &dmg(60));
        assert!(m.advance(4000).is_empty());
    }

    #[test]
    fn cooldown_drops_equal_or_lower_but_not_escalations() {
        let mut m = armed(cfg());
        m.handle(0, &dmg(60));
        assert_eq!(m.advance(300).len(), 1); // heavy at t=300
        m.handle(400, &dmg(30));
        assert!(m.advance(700).is_empty(), "medium inside cooldown is dropped");

        let mut m = armed(cfg());
        m.handle(0, &dmg(30));
        assert_eq!(m.advance(300).len(), 1); // medium
        m.handle(400, &dmg(60));
        assert_eq!(m.advance(700).len(), 1, "heavy breaks through the cooldown");
    }

    #[test]
    fn limits_clamp_everything() {
        let mut c = cfg();
        c.limits.max_intensity = 10;
        c.limits.max_duration_ms = 500;
        let mut m = armed(c);
        let s = m.handle(0, &Event::Death).pop().unwrap();
        assert_eq!((s.intensity, s.duration_ms), (10, 500));
    }
}
