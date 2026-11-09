use crate::config::{Config, Target};
use crate::mapper::Shock;
use crate::report::{Level, Outcome, Reporter, UiEvent};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// OpenShock's minimum command duration in ms (check the API docs if this changes).
const MIN_DURATION_MS: u32 = 300;

pub struct Flags {
    pub shared_fate: AtomicBool,
    pub paused: AtomicBool,
}

#[derive(Clone)]
pub struct Dispatcher {
    client: reqwest::Client,
    cfg: Arc<Config>,
    flags: Arc<Flags>,
    dry_run: bool,
    report: Reporter,
}

#[allow(clippy::too_many_arguments)]
fn delivery(
    shock: &Shock,
    target: &str,
    intensity: u8,
    duration_ms: u32,
    outcome: Outcome,
    detail: String,
) -> UiEvent {
    UiEvent::Delivery {
        tier: shock.tier.clone(),
        target: target.to_string(),
        kind: shock.kind.api_name().to_string(),
        intensity,
        duration_ms,
        reason: shock.reason.clone(),
        outcome,
        detail,
    }
}

impl Dispatcher {
    pub fn new(cfg: Arc<Config>, flags: Arc<Flags>, dry_run: bool, report: Reporter) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(cfg.api.timeout_ms))
            .user_agent(concat!("ecliptishock/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Dispatcher { client, cfg, flags, dry_run, report })
    }

    /// Shared fate on: every enabled target. Off: only the targets marked is_me.
    pub fn active_targets(&self) -> Vec<&Target> {
        let shared = self.flags.shared_fate.load(Ordering::Relaxed);
        self.cfg.targets.iter().filter(|t| t.enabled && (shared || t.is_me)).collect()
    }

    fn effective(&self, t: &Target, s: &Shock) -> (u8, u32) {
        let intensity = s
            .intensity
            .min(self.cfg.limits.max_intensity)
            .min(t.max_intensity.unwrap_or(100))
            .min(100);
        let duration = s
            .duration_ms
            .min(self.cfg.limits.max_duration_ms)
            .min(t.max_duration_ms.unwrap_or(u32::MAX))
            .max(MIN_DURATION_MS);
        (intensity, duration)
    }

    /// Fan the shock out to every active target in parallel; one failing key never blocks the rest.
    pub fn send(&self, shock: &Shock) {
        if self.flags.paused.load(Ordering::Relaxed) {
            (self.report)(delivery(shock, "-", 0, 0, Outcome::Paused, "paused, dropped".into()));
            return;
        }
        let targets = self.active_targets();
        if targets.is_empty() {
            (self.report)(UiEvent::Notice {
                level: Level::Warn,
                text: format!(
                    "{} fired but there are no active targets (mark one 'me', or turn shared fate on)",
                    shock.tier
                ),
            });
            return;
        }
        for t in targets {
            let (intensity, duration) = self.effective(t, shock);
            if self.dry_run {
                (self.report)(delivery(shock, &t.name, intensity, duration, Outcome::DryRun, String::new()));
                continue;
            }
            // Verify this body shape against the current OpenShock API docs before relying on it.
            let shocks: Vec<_> = t
                .shockers
                .iter()
                .map(|id| {
                    json!({
                        "id": id,
                        "type": shock.kind.api_name(),
                        "intensity": intensity,
                        "duration": duration,
                        "exclusive": true
                    })
                })
                .collect();
            let body = json!({ "shocks": shocks, "customName": self.cfg.api.custom_name });
            let url = format!("{}/2/shockers/control", self.cfg.api.base_url.trim_end_matches('/'));
            let client = self.client.clone();
            let token = t.token.clone();
            let name = t.name.clone();
            let shock = shock.clone();
            let report = self.report.clone();
            tokio::spawn(async move {
                let ev = match client.post(url).header("OpenShockToken", token).json(&body).send().await {
                    Ok(r) if r.status().is_success() => {
                        delivery(&shock, &name, intensity, duration, Outcome::Sent, format!("HTTP {}", r.status()))
                    }
                    Ok(r) => {
                        let status = r.status();
                        let mut text = r.text().await.unwrap_or_default();
                        text.truncate(200);
                        delivery(&shock, &name, intensity, duration, Outcome::Failed, format!("HTTP {status} {text}"))
                    }
                    Err(e) => delivery(&shock, &name, intensity, duration, Outcome::Failed, e.to_string()),
                };
                report(ev);
            });
        }
    }
}
