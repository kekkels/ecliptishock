//! The live loop: tail the VRChat log, run events through the mapper, fan shocks out.
//! Front ends (CLI, Tauri) only send `Control` messages in and receive `UiEvent`s out.
use crate::config::Config;
use crate::dispatch::{Dispatcher, Flags};
use crate::events::{parse_line, Event};
use crate::mapper::Mapper;
use crate::report::{Level, Reporter, Status, UiEvent};
use crate::tail::{find_log_dir, Tailer};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

pub enum Control {
    /// Swap in a new config (tiers, targets, limits...). Fight state is rebuilt from the log.
    SetConfig(Box<Config>),
    SetShared(bool),
    SetPaused(bool),
    TogglePaused,
    /// Fire a tier by name (or "death") using the current config and targets.
    Test(String),
    Stop,
}

#[derive(Clone)]
pub struct EngineHandle {
    tx: UnboundedSender<Control>,
}

impl EngineHandle {
    pub fn send(&self, c: Control) {
        let _ = self.tx.send(c);
    }
}

pub struct StartOpts {
    pub cfg: Config,
    /// Overrides the config's log folder (CLI --log-dir).
    pub log_dir: Option<PathBuf>,
    /// Overrides the config's shared_fate at startup.
    pub shared_override: Option<bool>,
    /// Forces dry-run regardless of the config (CLI --dry-run).
    pub force_dry_run: bool,
}

pub fn channel() -> (EngineHandle, UnboundedReceiver<Control>) {
    let (tx, rx) = unbounded_channel();
    (EngineHandle { tx }, rx)
}

/// Runs until `Control::Stop` or every handle is dropped. Must be polled on a tokio runtime.
pub async fn run(opts: StartOpts, report: Reporter, mut rx: UnboundedReceiver<Control>) {
    let mut eng = match Engine::new(opts, report.clone()) {
        Ok(e) => e,
        Err(e) => {
            report(UiEvent::Notice { level: Level::Error, text: format!("engine failed to start: {e}") });
            return;
        }
    };
    let mut tick = tokio::time::interval(Duration::from_millis(50));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = tick.tick() => eng.on_tick(),
            ctl = rx.recv() => match ctl {
                None | Some(Control::Stop) => break,
                Some(c) => eng.apply(c),
            },
        }
    }
}

struct Engine {
    cfg: Config,
    log_dir_override: Option<PathBuf>,
    force_dry_run: bool,
    report: Reporter,
    flags: Arc<Flags>,
    disp: Dispatcher,
    mapper: Mapper,
    tail: Option<Tailer>,
    last_attach: Option<Instant>,
    error: Option<String>,
    read_err: Option<String>,
    last_status: Option<Status>,
    t0: Instant,
}

impl Engine {
    fn new(opts: StartOpts, report: Reporter) -> anyhow::Result<Self> {
        let shared = opts.shared_override.unwrap_or(opts.cfg.group.shared_fate);
        let flags = Arc::new(Flags { shared_fate: AtomicBool::new(shared), paused: AtomicBool::new(false) });
        let dry = opts.cfg.dry_run || opts.force_dry_run;
        let disp = Dispatcher::new(Arc::new(opts.cfg.clone()), flags.clone(), dry, report.clone())?;
        Ok(Engine {
            mapper: Mapper::new(opts.cfg.clone()),
            cfg: opts.cfg,
            log_dir_override: opts.log_dir,
            force_dry_run: opts.force_dry_run,
            report,
            flags,
            disp,
            tail: None,
            last_attach: None,
            error: None,
            read_err: None,
            last_status: None,
            t0: Instant::now(),
        })
    }

    fn notice(&self, level: Level, text: String) {
        (self.report)(UiEvent::Notice { level, text });
    }

    fn apply(&mut self, c: Control) {
        match c {
            Control::SetShared(v) => self.flags.shared_fate.store(v, Ordering::Relaxed),
            Control::SetPaused(v) => self.flags.paused.store(v, Ordering::Relaxed),
            Control::TogglePaused => {
                let v = !self.flags.paused.load(Ordering::Relaxed);
                self.flags.paused.store(v, Ordering::Relaxed);
            }
            Control::Test(name) => match self.mapper.tier_shock(&name) {
                Some(s) => self.disp.send(&s),
                None => self.notice(Level::Warn, format!("no tier named '{name}'")),
            },
            Control::SetConfig(cfg) => self.reconfigure(*cfg),
            Control::Stop => {}
        }
        self.emit_status();
    }

    fn reconfigure(&mut self, cfg: Config) {
        self.flags.shared_fate.store(cfg.group.shared_fate, Ordering::Relaxed);
        let dry = cfg.dry_run || self.force_dry_run;
        match Dispatcher::new(Arc::new(cfg.clone()), self.flags.clone(), dry, self.report.clone()) {
            Ok(d) => self.disp = d,
            Err(e) => {
                self.notice(Level::Error, format!("could not apply settings: {e}"));
                return;
            }
        }
        self.mapper = Mapper::new(cfg.clone());
        self.cfg = cfg;
        // Re-attach so the (possibly changed) log folder is used and fight state is rebuilt.
        self.tail = None;
        self.last_attach = None;
    }

    fn on_tick(&mut self) {
        let now = self.t0.elapsed().as_millis() as u64;
        if self.tail.is_none() {
            self.try_attach();
        }
        let polled = self.tail.as_mut().map(|t| {
            let r = t.poll();
            (r, t.take_switched())
        });
        match polled {
            Some((Ok(lines), switched)) => {
                self.read_err = None;
                if switched {
                    self.mapper.reset();
                }
                for line in lines {
                    self.handle_line(now, &line);
                }
            }
            Some((Err(e), _)) => {
                let msg = e.to_string();
                if self.read_err.as_deref() != Some(msg.as_str()) {
                    self.notice(Level::Warn, format!("log read error: {msg}"));
                    self.read_err = Some(msg);
                }
            }
            None => {}
        }
        for s in self.mapper.advance(now) {
            self.disp.send(&s);
        }
        self.emit_status();
    }

    fn handle_line(&mut self, now: u64, line: &str) {
        let Some(p) = parse_line(line) else { return };
        match &p.event {
            Event::Damage { amount, source } => (self.report)(UiEvent::Hit {
                amount: *amount,
                source: source.clone(),
                tick: *amount <= self.cfg.damage.tick_max,
            }),
            Event::Death => (self.report)(UiEvent::Death),
            Event::Arm | Event::Disarm => {}
        }
        let before = self.mapper.is_armed();
        let shocks = self.mapper.handle(now, &p.event);
        let after = self.mapper.is_armed();
        if before != after {
            (self.report)(UiEvent::Fight { armed: after });
        }
        for s in shocks {
            self.disp.send(&s);
        }
    }

    fn try_attach(&mut self) {
        if self.last_attach.map_or(false, |t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        self.last_attach = Some(Instant::now());
        let dir = self
            .log_dir_override
            .clone()
            .or_else(|| self.cfg.log.dir.clone())
            .or_else(find_log_dir);
        let Some(dir) = dir else {
            self.error = Some("VRChat log folder not found - set it under Advanced".into());
            return;
        };
        if !dir.is_dir() {
            self.error = Some(format!("log folder does not exist: {}", dir.display()));
            return;
        }
        let mut t = Tailer::new(dir);
        match t.prime() {
            Ok(lines) => {
                for l in lines {
                    if let Some(p) = parse_line(&l) {
                        self.mapper.prime(&p.event);
                    }
                }
                self.error = if t.current().is_none() {
                    Some("no output_log file in the folder yet - start VRChat".into())
                } else {
                    None
                };
                self.tail = Some(t);
            }
            Err(e) => self.error = Some(format!("cannot read the log: {e}")),
        }
    }

    fn status(&self) -> Status {
        let cur = self.tail.as_ref().and_then(|t| t.current());
        Status {
            attached: cur.is_some(),
            log_dir: self.tail.as_ref().map(|t| t.dir().display().to_string()),
            log_file: cur.and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()),
            armed: self.mapper.is_armed(),
            shared_fate: self.flags.shared_fate.load(Ordering::Relaxed),
            paused: self.flags.paused.load(Ordering::Relaxed),
            dry_run: self.cfg.dry_run || self.force_dry_run,
            active_targets: self.disp.active_targets().len(),
            total_targets: self.cfg.targets.len(),
            error: self.error.clone(),
        }
    }

    fn emit_status(&mut self) {
        let s = self.status();
        if self.last_status.as_ref() != Some(&s) {
            self.last_status = Some(s.clone());
            (self.report)(UiEvent::Status(s));
        }
    }
}
