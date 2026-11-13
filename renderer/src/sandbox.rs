//! Runs the TeX tools (latex, dvipng, dvips, gs) on text from model output.
//! Safety comes from isolation, not from filtering commands: each run gets
//! a fresh private folder (deleted after), paranoid file access
//! (openin_any=p, openout_any=p: no absolute paths, no parent folders, no
//! dot files), no shell escape, a wall-clock limit, a memory limit, a cap on
//! the size of any file it writes and on the folder as a whole.

use std::fs::File;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub const MEMORY: u64 = 1 << 30; // address space of one run
pub const FILE_MAX: u64 = 16 * 1024 * 1024; // largest file a run may write
pub const FOLDER_MAX: u64 = 64 * 1024 * 1024; // all the files of a run together
const POLL: Duration = Duration::from_millis(5);
const SIZE_EVERY: Duration = Duration::from_millis(50);

/// A private folder for one TeX run, removed with everything in it when dropped.
pub struct Workdir {
    pub path: PathBuf,
}

impl Workdir {
    pub fn new() -> std::io::Result<Self> {
        static COUNT: AtomicU32 = AtomicU32::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let name = format!(
            "typeset-{}-{}-{nanos}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Workdir { path })
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Exited with this status.
    Exited(i32),
    /// Killed by a signal: the memory or file size limit, most often.
    Signalled,
    TimedOut,
    /// Its files grew past FOLDER_MAX.
    TooMuchOutput,
    NotStarted,
}

impl Outcome {
    pub fn ok(self) -> bool {
        self == Outcome::Exited(0)
    }
}

/// In words, for an error a user reads.
impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Exited(code) => write!(f, "exited {code}"),
            Outcome::Signalled => f.write_str("stopped at its memory, file size or CPU limit"),
            Outcome::TimedOut => f.write_str("timed out"),
            Outcome::TooMuchOutput => f.write_str("wrote too much"),
            Outcome::NotStarted => f.write_str("could not start"),
        }
    }
}

/// The environment of every run: paranoid file access, no shell escape,
/// and no font building (a missing glyph must fail at once).
fn tex_env(command: &mut Command) {
    for name in ["TEXINPUTS", "TEXMFOUTPUT", "TEXMFCNF", "TEXFORMATS"] {
        command.env_remove(name);
    }
    command.envs([
        ("openin_any", "p"),
        ("openout_any", "p"),
        ("shell_escape", "f"),
        ("max_print_line", "10000"),
        ("error_line", "254"),
        ("half_error_line", "238"),
        ("MKTEXTFM", "0"),
        ("MKTEXMF", "0"),
    ]);
}

fn folder_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .map(|meta| if meta.is_dir() { 0 } else { meta.len() })
        .sum()
}

/// Runs `argv` in `dir` (stdout to the file `stdout` there, or nowhere) and
/// waits at most `timeout`; on a timeout or too much output the run and
/// anything it started are killed.
pub fn run(argv: &[&str], dir: &Path, timeout: Duration, stdout: Option<&str>) -> Outcome {
    let Some((program, args)) = argv.split_first() else {
        return Outcome::NotStarted;
    };
    let out = match stdout {
        Some(name) => match File::create(dir.join(name)) {
            Ok(file) => Stdio::from(file),
            Err(_) => return Outcome::NotStarted,
        },
        None => Stdio::null(),
    };
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(Stdio::null());
    tex_env(&mut command);
    let cpu = timeout.as_secs() + 2;
    // SAFETY: between fork and exec only async-signal-safe calls (setsid,
    // setrlimit), on values computed before the fork.
    unsafe {
        command.pre_exec(move || {
            libc::setsid(); // its own group, so a kill reaches what it starts
            let limit = |resource, value: u64| {
                let lim = libc::rlimit {
                    rlim_cur: value as libc::rlim_t,
                    rlim_max: value as libc::rlim_t,
                };
                libc::setrlimit(resource, &lim)
            };
            limit(libc::RLIMIT_AS, MEMORY);
            limit(libc::RLIMIT_FSIZE, FILE_MAX);
            limit(libc::RLIMIT_CPU, cpu);
            limit(libc::RLIMIT_CORE, 0);
            Ok(())
        });
    }
    let Ok(mut child) = command.spawn() else {
        return Outcome::NotStarted;
    };
    let group = child.id() as libc::pid_t;
    let kill = |child: &mut std::process::Child| {
        // SAFETY: a plain kill(2) of the group the child leads.
        unsafe { libc::kill(-group, libc::SIGKILL) };
        let _ = child.kill();
        let _ = child.wait();
    };
    let start = Instant::now();
    let mut measured = start;
    loop {
        match has_ended(group) {
            Some(true) => {
                // Anything it left running goes with it. The child is not
                // reaped yet, so its number, the group's, cannot have gone
                // to another process.
                // SAFETY: as above.
                unsafe { libc::kill(-group, libc::SIGKILL) };
                return match child.wait().map(|status| status.code()) {
                    Ok(Some(code)) => Outcome::Exited(code),
                    _ => Outcome::Signalled,
                };
            }
            Some(false) => {}
            None => {
                kill(&mut child);
                return Outcome::Signalled;
            }
        }
        if start.elapsed() >= timeout {
            kill(&mut child);
            return Outcome::TimedOut;
        }
        if measured.elapsed() >= SIZE_EVERY {
            measured = Instant::now();
            if folder_size(dir) > FOLDER_MAX {
                kill(&mut child);
                return Outcome::TooMuchOutput;
            }
        }
        std::thread::sleep(POLL);
    }
}

/// Whether the child `pid` has ended, leaving it to be reaped (None when
/// that cannot be asked).
fn has_ended(pid: libc::pid_t) -> Option<bool> {
    // SAFETY: an all-zero siginfo_t is valid; waitid fills it in, and with
    // WNOWAIT leaves the child waitable.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    let status = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    // With WNOHANG, si_signo stays 0 while the child runs.
    (status == 0).then_some(info.si_signo != 0)
}

/// The start of a file the run wrote (at most `max` bytes), as text.
pub fn read_text(path: &Path, max: u64) -> Option<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    File::open(path).ok()?.take(max).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Whether `program` is an executable file in a folder of PATH.
pub fn on_path(program: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        std::fs::metadata(dir.join(program)).is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    })
}
