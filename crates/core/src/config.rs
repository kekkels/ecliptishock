use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Shock,
    Vibrate,
    Sound,
}

impl Kind {
    pub fn api_name(self) -> &'static str {
        match self {
            Kind::Shock => "Shock",
            Kind::Vibrate => "Vibrate",
            Kind::Sound => "Sound",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// Log what would be sent but never call the API. (Scalars first: TOML wants values before tables.)
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub api: Api,
    #[serde(default)]
    pub group: Group,
    #[serde(default)]
    pub log: LogCfg,
    #[serde(default)]
    pub damage: Damage,
    #[serde(default)]
    pub dot: Dot,
    #[serde(default = "default_tiers")]
    pub tiers: Vec<Tier>,
    #[serde(default)]
    pub death: Death,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Api {
    #[serde(default = "d_base")]
    pub base_url: String,
    #[serde(default = "d_name")]
    pub custom_name: String,
    #[serde(default = "d_timeout")]
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Group {
    /// true: your damage/death hits every enabled target. false: only targets with is_me = true.
    #[serde(default)]
    pub shared_fate: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogCfg {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<PathBuf>,
    /// Only fire between a stage/boss line and the next lobby/intermission line.
    #[serde(default = "d_true")]
    pub only_in_fight: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Damage {
    /// Hits at or below this are treated as DoT ticks.
    #[serde(default = "d_tick_max")]
    pub tick_max: u32,
    /// Real hits landing within this window of the first one merge into one shock.
    #[serde(default = "d_burst")]
    pub burst_ms: u64,
    /// Minimum gap between non-death shocks (a higher tier can still break through).
    #[serde(default = "d_cooldown")]
    pub cooldown_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dot {
    #[serde(default = "d_true")]
    pub enabled: bool,
    /// Flush the pooled ticks as ONE shock once no tick has arrived for this long.
    #[serde(default = "d_dot_quiet")]
    pub quiet_ms: u64,
    /// Safety valve: a DoT that never stops still flushes this often.
    #[serde(default = "d_dot_hold")]
    pub max_hold_ms: u64,
    /// Pooled total is multiplied by this before picking a tier.
    #[serde(default = "d_dot_scale")]
    pub scale: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tier {
    pub name: String,
    pub min_damage: u32,
    #[serde(default = "d_kind")]
    pub kind: Kind,
    pub intensity: u8,
    pub duration_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Death {
    #[serde(default = "d_kind")]
    pub kind: Kind,
    #[serde(default = "d_death_int")]
    pub intensity: u8,
    #[serde(default = "d_death_dur")]
    pub duration_ms: u32,
    /// The game prints the death line several times; repeats inside this window are ignored.
    #[serde(default = "d_death_debounce")]
    pub debounce_ms: u64,
    /// Damage is ignored for this long after a death.
    #[serde(default = "d_death_suppress")]
    pub suppress_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    /// Hard ceilings applied to everything, including death and `test`.
    #[serde(default = "d_max_int")]
    pub max_intensity: u8,
    #[serde(default = "d_max_dur")]
    pub max_duration_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub name: String,
    pub token: String,
    pub shockers: Vec<String>,
    #[serde(default = "d_true")]
    pub enabled: bool,
    /// Marks the local player's own target (used when shared fate is off).
    #[serde(default)]
    pub is_me: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_intensity: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u32>,
}

fn d_true() -> bool { true }
fn d_base() -> String { "https://api.openshock.app".into() }
fn d_name() -> String { "Ecliptica".into() }
fn d_timeout() -> u64 { 5000 }
fn d_tick_max() -> u32 { 7 }
fn d_burst() -> u64 { 250 }
fn d_cooldown() -> u64 { 700 }
fn d_dot_quiet() -> u64 { 1200 }
fn d_dot_hold() -> u64 { 4000 }
fn d_dot_scale() -> f64 { 0.5 }
fn d_kind() -> Kind { Kind::Shock }
fn d_death_int() -> u8 { 35 }
fn d_death_dur() -> u32 { 1500 }
fn d_death_debounce() -> u64 { 1500 }
fn d_death_suppress() -> u64 { 3000 }
fn d_max_int() -> u8 { 50 }
fn d_max_dur() -> u32 { 3000 }

// Tuned to a real session: 65% of damage lines are ticks (1-7); real hits are mostly 8-49,
// ~10% are 50-99, and only a handful ever reach 100+.
fn default_tiers() -> Vec<Tier> {
    vec![
        Tier { name: "light".into(), min_damage: 8, kind: Kind::Vibrate, intensity: 40, duration_ms: 300 },
        Tier { name: "medium".into(), min_damage: 25, kind: Kind::Shock, intensity: 12, duration_ms: 400 },
        Tier { name: "heavy".into(), min_damage: 50, kind: Kind::Shock, intensity: 25, duration_ms: 700 },
        Tier { name: "crushing".into(), min_damage: 100, kind: Kind::Shock, intensity: 40, duration_ms: 1000 },
    ]
}

macro_rules! impl_default {
    ($t:ty) => {
        impl Default for $t {
            fn default() -> Self {
                toml::from_str("").expect("defaults")
            }
        }
    };
}
impl_default!(Config);
impl_default!(Api);
impl_default!(LogCfg);
impl_default!(Damage);
impl_default!(Dot);
impl_default!(Death);
impl_default!(Limits);

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str::<Config>(&text)
            .with_context(|| format!("parsing {}", path.display()))?
            .finalize()
    }

    /// Atomic write; the file holds API tokens, so it is 0600 on Unix.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serializing config")?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    pub fn finalize(mut self) -> Result<Self> {
        if self.tiers.is_empty() {
            bail!("config needs at least one [[tiers]] entry");
        }
        self.tiers.sort_by_key(|t| t.min_damage);
        for t in &self.targets {
            if t.token.trim().is_empty() || t.shockers.is_empty() {
                bail!("target '{}' needs a token and at least one shocker id", t.name);
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_through_toml() {
        let c = Config::default().finalize().unwrap();
        let text = toml::to_string_pretty(&c).unwrap();
        let back = toml::from_str::<Config>(&text).unwrap().finalize().unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn targets_and_optionals_roundtrip() {
        let mut c = Config::default();
        c.dry_run = true;
        c.group.shared_fate = true;
        c.log.dir = Some("/tmp/x".into());
        c.dot.scale = 0.3;
        c.targets.push(Target {
            name: "friend".into(),
            token: "tok".into(),
            shockers: vec!["a".into(), "b".into()],
            enabled: true,
            is_me: false,
            max_intensity: Some(30),
            max_duration_ms: None,
        });
        let c = c.finalize().unwrap();
        let text = toml::to_string_pretty(&c).unwrap();
        assert_eq!(text.matches("max_duration_ms").count(), 1, "only [limits] has it; the target's None is omitted");
        assert_eq!(c, toml::from_str::<Config>(&text).unwrap().finalize().unwrap());
    }

    #[test]
    fn save_writes_a_private_file() {
        let dir = std::env::temp_dir().join(format!("eclip-cfg-{}", std::process::id()));
        let path = dir.join("nested").join("config.toml");
        Config::default().save(&path).unwrap();
        assert!(Config::load(&path).is_ok());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
