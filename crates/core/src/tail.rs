use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn default_log_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let tail = "compatdata/438100/pfx/drive_c/users/steamuser/AppData/LocalLow/VRChat/VRChat";
        for steam in [".local/share/Steam", ".steam/steam", ".var/app/com.valvesoftware.Steam/.local/share/Steam"] {
            v.push(home.join(steam).join("steamapps").join(tail));
        }
    }
    if let Some(up) = std::env::var_os("USERPROFILE").map(PathBuf::from) {
        v.push(up.join("AppData/LocalLow/VRChat/VRChat"));
    }
    v
}

pub fn find_log_dir() -> Option<PathBuf> {
    default_log_dirs().into_iter().find(|p| p.is_dir())
}

/// VRChat names logs output_log_YYYY-MM-DD_HH-MM-SS.txt, so the greatest name is the newest.
pub fn newest_log(dir: &Path) -> Option<PathBuf> {
    fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .map(|n| {
                    let n = n.to_string_lossy();
                    n.starts_with("output_log_") && n.ends_with(".txt")
                })
                .unwrap_or(false)
        })
        .max_by(|a, b| a.file_name().cmp(&b.file_name()))
}

pub struct Tailer {
    dir: PathBuf,
    path: Option<PathBuf>,
    offset: u64,
    buf: Vec<u8>,
    last_scan: Option<Instant>,
    switched: bool,
}

impl Tailer {
    pub fn new(dir: PathBuf) -> Self {
        Tailer { dir, path: None, offset: 0, buf: Vec::new(), last_scan: None, switched: false }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn current(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// True once after poll() moved to a newer log file (VRChat relaunched).
    pub fn take_switched(&mut self) -> bool {
        std::mem::take(&mut self.switched)
    }

    /// Read the current newest log from the start (to rebuild fight state) and park at its end.
    pub fn prime(&mut self) -> io::Result<Vec<String>> {
        self.path = newest_log(&self.dir);
        self.last_scan = Some(Instant::now());
        self.offset = 0;
        self.buf.clear();
        self.switched = false;
        self.read_new()
    }

    /// New complete lines since the last call. Switches to a newer log file when VRChat relaunches.
    pub fn poll(&mut self) -> io::Result<Vec<String>> {
        let stale = self.last_scan.map_or(true, |t| t.elapsed() > Duration::from_secs(2));
        if stale {
            self.last_scan = Some(Instant::now());
            let newest = newest_log(&self.dir);
            if newest.is_some() && newest != self.path {
                self.path = newest;
                self.offset = 0;
                self.buf.clear();
                self.switched = true;
            }
        }
        self.read_new()
    }

    fn read_new(&mut self) -> io::Result<Vec<String>> {
        let Some(path) = &self.path else { return Ok(Vec::new()) };
        let mut f = File::open(path)?;
        let len = f.metadata()?.len();
        if len < self.offset {
            self.offset = 0;
            self.buf.clear();
        }
        if len == self.offset {
            return Ok(Vec::new());
        }
        f.seek(SeekFrom::Start(self.offset))?;
        let mut chunk = Vec::new();
        f.take(len - self.offset).read_to_end(&mut chunk)?;
        self.offset = len;
        self.buf.extend(chunk);

        let mut lines = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            lines.push(String::from_utf8_lossy(&line).trim_end_matches(['\r', '\n']).to_string());
        }
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn tails_partial_lines_and_follows_new_files() {
        let dir = std::env::temp_dir().join(format!("ecliptishock-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("output_log_2026-11-06_01-00-00.txt");
        fs::write(&a, "old line\n").unwrap();

        let mut t = Tailer::new(dir.clone());
        assert_eq!(t.prime().unwrap(), vec!["old line"]);
        assert!(t.poll().unwrap().is_empty());

        let mut f = fs::OpenOptions::new().append(true).open(&a).unwrap();
        write!(f, "half").unwrap();
        assert!(t.poll().unwrap().is_empty(), "incomplete line is held back");
        write!(f, " done\r\nnext\n").unwrap();
        assert_eq!(t.poll().unwrap(), vec!["half done", "next"]);

        let b = dir.join("output_log_2026-11-06_02-00-00.txt");
        fs::write(&b, "fresh\n").unwrap();
        t.last_scan = None; // force a rescan
        assert_eq!(t.poll().unwrap(), vec!["fresh"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
