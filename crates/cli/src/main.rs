use anyhow::{Context, Result};
use clap::Parser;
use ecliptishock_core::config::Config;
use ecliptishock_core::engine::{self, Control, EngineHandle, StartOpts};
use ecliptishock_core::events::{parse_line, Event};
use ecliptishock_core::mapper::{Mapper, Shock};
use ecliptishock_core::report::{Reporter, Status, UiEvent};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Parser)]
#[command(version, about = "Ecliptica (VRChat) damage/death -> OpenShock, with shared-fate group mode")]
struct Args {
    /// Config file
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,
    /// Parse the live log but only print what would be sent
    #[arg(long)]
    dry_run: bool,
    /// Replay a saved log through the mapper (never sends anything) and print a summary
    #[arg(long, value_name = "FILE")]
    replay: Option<PathBuf>,
    /// Start with shared fate on (overrides the config)
    #[arg(long, conflicts_with = "solo")]
    shared_fate: bool,
    /// Start with shared fate off (overrides the config)
    #[arg(long)]
    solo: bool,
    /// VRChat log directory (overrides the config and auto-detection)
    #[arg(long)]
    log_dir: Option<PathBuf>,
    /// Also print every damage hit, death and fight change
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    if let Some(file) = &args.replay {
        let cfg = if args.config.exists() { Config::load(&args.config)? } else { Config::default().finalize()? };
        return replay(cfg, file);
    }
    if !args.config.exists() {
        anyhow::bail!("{} not found - copy config.example.toml to config.toml and fill in your targets", args.config.display());
    }
    live(Config::load(&args.config)?, &args).await
}

fn hms(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}:{:02}", (s / 3600) % 24, (s / 60) % 60, s % 60)
}

fn replay(cfg: Config, file: &Path) -> Result<()> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let mut mapper = Mapper::new(cfg);
    let mut last = 0u64;
    let mut dmg_lines = 0u32;
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let report = |ts: u64, s: &Shock, counts: &mut BTreeMap<String, u32>| {
        *counts.entry(s.tier.clone()).or_default() += 1;
        println!("{}  {:<8} {} {}% {}ms  ({})", hms(ts), s.tier, s.kind.api_name(), s.intensity, s.duration_ms, s.reason);
    };
    for line in text.lines() {
        let Some(p) = parse_line(line) else { continue };
        if let Some(ts) = p.ts_ms {
            last = ts;
        }
        if matches!(p.event, Event::Damage { .. }) {
            dmg_lines += 1;
        }
        for s in mapper.handle(last, &p.event) {
            report(last, &s, &mut counts);
        }
    }
    for s in mapper.advance(last + 3_600_000) {
        report(last, &s, &mut counts);
    }
    let total: u32 = counts.values().sum();
    println!("\n{dmg_lines} damage lines -> {total} shocks: {counts:?}");
    println!("(replay uses the log's 1-second timestamps, so burst/DoT windows are coarser than live)");
    Ok(())
}

async fn live(cfg: Config, args: &Args) -> Result<()> {
    let shared_override = if args.shared_fate {
        Some(true)
    } else if args.solo {
        Some(false)
    } else {
        None
    };
    let status: Arc<Mutex<Option<Status>>> = Arc::new(Mutex::new(None));
    let st = status.clone();
    let verbose = args.verbose;
    let report: Reporter = Arc::new(move |ev: UiEvent| {
        match &ev {
            UiEvent::Status(s) => *st.lock().unwrap() = Some(s.clone()),
            UiEvent::Hit { .. } | UiEvent::Death | UiEvent::Fight { .. } if !verbose => return,
            _ => {}
        }
        println!("{ev}");
    });
    let (handle, rx) = engine::channel();
    let task = tokio::spawn(engine::run(
        StartOpts { cfg, log_dir: args.log_dir.clone(), shared_override, force_dry_run: args.dry_run },
        report,
        rx,
    ));
    println!("commands: shared [on|off], pause, resume, status, test [tier|death], quit  (Ctrl+C also quits)\n");

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdin_open = true;
    loop {
        tokio::select! {
            line = lines.next_line(), if stdin_open => match line {
                Ok(Some(l)) => if !command(&l, &handle, &status) { break },
                _ => stdin_open = false,
            },
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    handle.send(Control::Stop);
    let _ = task.await;
    Ok(())
}

fn command(cmd: &str, handle: &EngineHandle, status: &Mutex<Option<Status>>) -> bool {
    let mut it = cmd.split_whitespace();
    match it.next() {
        Some("shared") => {
            let on = match it.next() {
                Some("on") => true,
                Some("off") => false,
                _ => !status.lock().unwrap().as_ref().map_or(false, |s| s.shared_fate),
            };
            handle.send(Control::SetShared(on));
        }
        Some("pause") => handle.send(Control::SetPaused(true)),
        Some("resume") => handle.send(Control::SetPaused(false)),
        Some("status") => match status.lock().unwrap().as_ref() {
            Some(s) => println!("{}", UiEvent::Status(s.clone())),
            None => println!("no status yet"),
        },
        Some("test") => handle.send(Control::Test(it.next().unwrap_or("light").to_string())),
        Some("quit") | Some("exit") => return false,
        _ => println!("commands: shared [on|off], pause, resume, status, test [tier|death], quit"),
    }
    true
}
