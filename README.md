# ecliptishock

Turns **Ecliptica** (VRChat) damage and deaths into OpenShock commands, by reading the VRChat output log.

```
crates/core   parser, damage mapper, dispatcher and the live engine (no UI)
crates/cli    terminal front end            -> `ecliptishock`
src-tauri     desktop app (Tauri)           -> `ecliptishock-app`
ui/           the app's settings page + live feed (plain HTML/CSS/JS, no build step)
```

## Desktop app

Settings page (tiers, DoT pooling, limits, targets, log folder), a live feed of hits, deaths and
shocks, header toggles for **Shared fate / Dry run / Paused**, a Test button per tier, and a tray
icon. Edits apply live when you press *Save & apply*. Closing the window hides it to the tray (if
your desktop has one) so shocks keep working; quit from the tray menu.

Arch / CachyOS prerequisites (from Tauri's prerequisites page, double-check there):

```
sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file openssl appmenu-gtk-module libappindicator-gtk3 librsvg xdotool
```

Build:

```
cargo build --release -p ecliptishock-app --features custom-protocol   # just the binary
# or, for installers (AppImage/deb/rpm):  cargo install tauri-cli --version "^2" --locked && cargo tauri build
./target/release/ecliptishock-app
```

Settings live in `~/.config/dev.ecliptishock.app/config.toml` (mode 600, holds tokens). Override with the
`ECLIPTISHOCK_CONFIG` env var. A blank window on some Wayland/NVIDIA setups is usually fixed with
`WEBKIT_DISABLE_DMABUF_RENDERER=1`. Windows needs WebView2 (already present on Windows 10/11); build the
Windows installer on Windows or CI (`.github/workflows/build.yml`, untested) - cross-compiling Tauri is painful.

## CLI

```
cargo build --release -p ecliptishock
cp config.example.toml config.toml
ecliptishock --replay output_log.txt   # what WOULD it have done? (sends nothing)
ecliptishock --dry-run                 # live log, prints only
ecliptishock                           # live, sends
```

Commands while running: `shared [on|off]`, `pause`, `resume`, `status`, `test [tier|death]`, `quit`.
Cross-compile the Windows CLI from Linux: `cargo build --release -p ecliptishock --target x86_64-pc-windows-gnu`
(needs `mingw-w64-gcc` and `rustup target add x86_64-pc-windows-gnu`).

## How it behaves

- **Tiers**: a hit (or hits landing within `burst_ms`) picks the highest tier whose `min_damage` it reaches. Death is its own profile.
- **DoT**: hits <= `tick_max` pool up and fire as ONE shock when ticks stop (`quiet_ms`), or every `max_hold_ms` if they never do. Pooled total x `scale` picks the tier.
- **Shared fate**: on = your damage/death shocks every enabled target; off = only targets marked "me".
  For a group, everyone runs a copy with the same target list and shared fate on. Each copy only reacts to its own
  player's log, so one hit fires once, from one machine, to everyone. No server needed.
- **Safety**: global `[limits]` and per-target caps clamp everything (including Test). Nothing fires outside a fight.
  Pausing drops events rather than queueing them. Use scoped, expiring tokens for friends.
- The control request (`POST /2/shockers/control`, `OpenShockToken` header) is written from memory of the OpenShock API:
  confirm it against the current docs, and use Dry run first.

## Tests

`cargo test --workspace` - includes a replay of a real session (6 deaths) and an end-to-end engine test.
