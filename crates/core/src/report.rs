//! Everything the engine tells a front end (CLI printer or Tauri window).
use serde::Serialize;
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Status {
    /// A log file is being tailed.
    pub attached: bool,
    pub log_dir: Option<String>,
    pub log_file: Option<String>,
    /// Inside a fight (a stage/boss line was seen and no lobby/intermission since).
    pub armed: bool,
    pub shared_fate: bool,
    pub paused: bool,
    pub dry_run: bool,
    pub active_targets: usize,
    pub total_targets: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    DryRun,
    Sent,
    Failed,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEvent {
    Status(Status),
    Hit { amount: u32, source: String, tick: bool },
    Death,
    Fight { armed: bool },
    Delivery {
        tier: String,
        target: String,
        kind: String,
        intensity: u8,
        duration_ms: u32,
        reason: String,
        outcome: Outcome,
        detail: String,
    },
    Notice { level: Level, text: String },
}

pub type Reporter = Arc<dyn Fn(UiEvent) + Send + Sync>;

impl fmt::Display for UiEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiEvent::Status(s) => write!(
                f,
                "[status] {} | {} | shared fate {} | {}{}{} | targets {}/{}",
                s.log_file.as_deref().unwrap_or("no log yet"),
                if s.armed { "in a fight" } else { "idle" },
                if s.shared_fate { "ON" } else { "off" },
                if s.paused { "PAUSED " } else { "" },
                if s.dry_run { "dry-run" } else { "live" },
                s.error.as_ref().map(|e| format!(" | {e}")).unwrap_or_default(),
                s.active_targets,
                s.total_targets
            ),
            UiEvent::Hit { amount, source, tick } => {
                write!(f, "[hit] {}{amount} {source}", if *tick { "tick " } else { "" })
            }
            UiEvent::Death => write!(f, "[death]"),
            UiEvent::Fight { armed } => write!(f, "[fight] {}", if *armed { "started" } else { "ended" }),
            UiEvent::Delivery { tier, target, kind, intensity, duration_ms, reason, outcome, detail } => {
                let tag = match outcome {
                    Outcome::DryRun => "dry-run",
                    Outcome::Sent => "sent",
                    Outcome::Failed => "FAIL",
                    Outcome::Paused => "paused",
                };
                write!(f, "[{tag}] {tier} -> {target}: {kind} {intensity}% {duration_ms}ms ({reason})")?;
                if !detail.is_empty() {
                    write!(f, " {detail}")?;
                }
                Ok(())
            }
            UiEvent::Notice { level, text } => {
                let l = match level {
                    Level::Info => "info",
                    Level::Warn => "warn",
                    Level::Error => "error",
                };
                write!(f, "[{l}] {text}")
            }
        }
    }
}
