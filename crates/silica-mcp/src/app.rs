//! The app under control: where it lives, how it is started, and the socket
//! client that drives it.
//!
//! One child process at a time, owned by this server. Everything a user might
//! need to point elsewhere — the socket, the binary, the screenshot command,
//! the per-call timeout — comes from the environment, because an MCP server is
//! configured in a `.mcp.json` entry and that is the only place a user can
//! reach it.

use serde_json::{json, Value};
use silica_control::Client;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long the app may take to bind its socket after a spawn. A cold GPU
/// context plus the vault scan is seconds, not milliseconds.
const SOCKET_WAIT: Duration = Duration::from_secs(30);
/// `wait_loaded`'s budget on launch, and the read timeout around it.
const LOAD_WAIT: Duration = Duration::from_secs(30);
/// Attaching to an app somebody else started: either it is listening now or
/// there is nothing to talk to.
const ATTACH_WAIT: Duration = Duration::from_millis(300);
/// How long a polite `quit` has to land before the child is killed.
const QUIT_GRACE: Duration = Duration::from_secs(3);

/// What `silica_screenshot` runs without `SILICA_MCP_SCREENSHOT`: KDE's
/// spectacle capturing the active window.
#[cfg(unix)]
const DEFAULT_SCREENSHOT: &str = "spectacle -b -n -a -o {path}";
/// What `silica_screenshot` runs without `SILICA_MCP_SCREENSHOT` on Windows:
/// PowerShell copying the foreground window off the screen. The process is
/// made per-monitor DPI aware first so the rectangle and the pixels agree,
/// and the rectangle is DWM's visible frame rather than `GetWindowRect`'s,
/// which includes the invisible resize border.
#[cfg(windows)]
const DEFAULT_SCREENSHOT: &str = "\
Add-Type -AssemblyName System.Drawing; \
Add-Type -Namespace Silica -Name Shot -MemberDefinition '\
[DllImport(\"user32.dll\")] public static extern bool SetProcessDpiAwarenessContext(IntPtr context); \
[DllImport(\"user32.dll\")] public static extern IntPtr GetForegroundWindow(); \
[DllImport(\"user32.dll\")] public static extern bool GetWindowRect(IntPtr window, out RECT rect); \
[DllImport(\"dwmapi.dll\")] public static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out RECT rect, int size); \
[StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }'; \
[Silica.Shot]::SetProcessDpiAwarenessContext([IntPtr](-4)) | Out-Null; \
$window = [Silica.Shot]::GetForegroundWindow(); \
$rect = New-Object Silica.Shot+RECT; \
if ([Silica.Shot]::DwmGetWindowAttribute($window, 9, [ref]$rect, 16) -ne 0) { [Silica.Shot]::GetWindowRect($window, [ref]$rect) | Out-Null }; \
$bitmap = New-Object System.Drawing.Bitmap ($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top); \
$graphics = [System.Drawing.Graphics]::FromImage($bitmap); \
$graphics.CopyFromScreen($rect.Left, $rect.Top, 0, 0, $bitmap.Size); \
$bitmap.Save('{path}', [System.Drawing.Imaging.ImageFormat]::Png)";

/// Everything read from the environment, once, at startup.
pub struct Config {
    /// `SILICA_MCP_SOCKET`.
    pub socket: PathBuf,
    /// `SILICA_MCP_BIN`: a plain path, or a command line (run through `sh -c`)
    /// with an optional `{args}` placeholder for the app's own arguments.
    pub bin: String,
    /// `SILICA_MCP_SCREENSHOT`, with `{path}` and optionally `{crop}`.
    pub screenshot: String,
    /// `SILICA_MCP_TIMEOUT_MS`.
    pub timeout: Duration,
    /// `SILICA_MCP_REPO`: the working directory the app is run from, and what
    /// a relative binary path is resolved against.
    pub repo: PathBuf,
}

impl Config {
    pub fn from_env() -> Config {
        let repo = env_path("SILICA_MCP_REPO").unwrap_or_else(default_repo);
        Config {
            socket: env_path("SILICA_MCP_SOCKET").unwrap_or_else(default_socket),
            bin: env_text("SILICA_MCP_BIN").unwrap_or_else(|| {
                repo.join("target/release")
                    .join(format!("silica{}", std::env::consts::EXE_SUFFIX))
                    .to_string_lossy()
                    .into_owned()
            }),
            screenshot: env_text("SILICA_MCP_SCREENSHOT")
                .unwrap_or_else(|| DEFAULT_SCREENSHOT.to_string()),
            timeout: Duration::from_millis(
                env_text("SILICA_MCP_TIMEOUT_MS")
                    .and_then(|ms| ms.parse().ok())
                    .unwrap_or(15_000),
            ),
            repo,
        }
    }

    /// A config that reaches one socket and can launch nothing: the tests run
    /// their own listener instead of an app.
    #[cfg(test)]
    pub fn for_test(socket: PathBuf) -> Config {
        Config {
            socket,
            bin: String::from("/nonexistent/silica"),
            screenshot: String::from("false"),
            timeout: Duration::from_secs(2),
            repo: std::env::temp_dir(),
        }
    }
}

fn env_text(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn env_path(name: &str) -> Option<PathBuf> {
    env_text(name).map(PathBuf::from)
}

fn default_socket() -> PathBuf {
    runtime_dir().join("silica-mcp.sock")
}

/// A directory only this user can enter, for the socket and the screenshots.
///
/// `XDG_RUNTIME_DIR` is already one. On Unix the fallback is the shared temp dir,
/// where both names are predictable: another local user who creates them first
/// owns the socket this server would then connect to — feeding the model a
/// fabricated `state` and collecting the vault paths and text the tools send —
/// and a planted symlink would redirect the capture. So the fallback gets a
/// private directory of its own instead.
fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => private_dir(),
    }
}

#[cfg(unix)]
fn private_dir() -> PathBuf {
    let base = std::env::temp_dir();
    let uid = our_uid();
    // The stable name first, so an app started by hand with `--control` can be
    // reached by its default socket. The others are for a name somebody else
    // owns: a pid nobody could have pre-empted, then a timestamped one in case
    // a dead run of ours left that behind too.
    let names = [
        match uid {
            Some(uid) => format!("silica-{uid}"),
            None => "silica-self".to_string(),
        },
        format!("silica-mcp-{}", std::process::id()),
        format!("silica-mcp-{}-{}", std::process::id(), nanos()),
    ];
    for name in &names {
        let candidate = base.join(name);
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(&candidate) {
            Ok(()) => return candidate,
            // Ours from an earlier run: keep using it.
            Err(e) if e.kind() == ErrorKind::AlreadyExists && is_ours(&candidate, uid) => {
                return candidate
            }
            Err(_) => {}
        }
    }
    // Nothing private could be had here. A path still goes back, so the bind
    // or the connect fails naming it, which is the honest error.
    base.join(&names[names.len() - 1])
}

/// On Windows the user's temp directory is already theirs alone — its ACL
/// names the user, SYSTEM and Administrators — and a socket there has no
/// mode bits to set anyway, so a folder of ours inside it is as private as
/// the platform gets. It is also short: `sun_path` holds 108 bytes.
#[cfg(windows)]
fn private_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("silica");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Nanoseconds of the current second: enough to make a name nobody watching
/// the temp dir can create in advance.
#[cfg(unix)]
fn nanos() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos())
        .unwrap_or_default()
}

/// Whether `path` is a directory of ours that nobody else can enter.
#[cfg(unix)]
fn is_ours(path: &Path, uid: Option<u32>) -> bool {
    let Some(uid) = uid else {
        // Without a uid to compare against, only a directory we just created
        // ourselves counts as private.
        return false;
    };
    match std::fs::symlink_metadata(path) {
        Ok(meta) => meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        Err(_) => false,
    }
}

/// This process's user id. `libc::getuid` is not a dependency here. On
/// Linux the kernel names the owner of `/proc/self` for us; where there is no
/// procfs (macOS, the BSDs) a directory we create is owned by us, so its
/// owner is the answer.
#[cfg(unix)]
fn our_uid() -> Option<u32> {
    if let Ok(meta) = std::fs::metadata("/proc/self") {
        return Some(meta.uid());
    }
    let probe = std::env::temp_dir().join(format!("silica-uid-{}-{}", std::process::id(), nanos()));
    let mut builder = std::fs::DirBuilder::new();
    builder.mode(0o700);
    builder.create(&probe).ok()?;
    let uid = std::fs::symlink_metadata(&probe)
        .ok()
        .map(|meta| meta.uid());
    let _ = std::fs::remove_dir(&probe);
    uid
}

/// `CARGO_MANIFEST_DIR` is `<repo>/crates/silica-mcp` when this binary is
/// built, which is the only hint it has about where the release build it
/// drives lives. `SILICA_MCP_REPO` is for anyone who moved either.
fn default_repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

/// The running app, or the absence of one.
pub struct App {
    config: Config,
    /// `Some` only for a child this server spawned; a user may also point it
    /// at an app they started with `--control` themselves.
    child: Option<Child>,
    client: Option<Client>,
}

impl App {
    pub fn new(config: Config) -> App {
        App {
            config,
            child: None,
            client: None,
        }
    }

    /// Spawn the app on a vault, wait for the socket, wait for the index, and
    /// describe what came up.
    pub fn launch(&mut self, arguments: &Value) -> Result<String, String> {
        if let Some(pid) = self.running_pid() {
            return Err(format!(
                "silica is already running (pid {pid}) on {}; call silica_quit first",
                self.config.socket.display()
            ));
        }
        let vault = arguments
            .get("vault")
            .and_then(Value::as_str)
            .ok_or("silica_launch needs `vault`: an absolute path to a vault folder")?;
        let root = std::fs::canonicalize(vault)
            .map(plain)
            .map_err(|e| format!("vault {vault}: {e}"))
            .and_then(|root| {
                if root.is_dir() {
                    Ok(root)
                } else {
                    Err(format!("vault {} is not a folder", root.display()))
                }
            })?;
        let mut argv = vec![
            root.display().to_string(),
            "--control".to_string(),
            self.config.socket.display().to_string(),
        ];
        if let Some(note) = arguments.get("note").and_then(Value::as_str) {
            argv.push("--note".to_string());
            argv.push(note.to_string());
        }
        match (number(arguments, "width"), number(arguments, "height")) {
            (Some(width), Some(height)) => {
                argv.push("--size".to_string());
                argv.push(format!("{width}x{height}"));
            }
            (None, None) => {}
            _ => return Err("give `width` and `height` together, or neither".to_string()),
        }

        // Only now the environment: a bad argument should not be reported as a
        // missing binary, and a missing binary should not have spawned.
        let shell = self.config.bin.contains(char::is_whitespace);
        if !shell && !Path::new(&self.config.bin).is_file() {
            return Err(format!(
                "no silica binary at {}: build it with `cargo build --release -p silica`, \
                 or set SILICA_MCP_BIN to the command that runs it",
                self.config.bin
            ));
        }
        let child = self.spawn(shell, &argv)?;
        let pid = child.id();
        self.child = Some(child);
        let client = match Client::connect(&self.config.socket, SOCKET_WAIT) {
            Ok(client) => client,
            Err(e) => {
                let gone = match self.reap(Duration::ZERO) {
                    Some(status) => format!(" (it exited {status}; its output is on stderr)"),
                    None => String::new(),
                };
                return Err(format!(
                    "silica never bound {}: {e}{gone}",
                    self.config.socket.display()
                ));
            }
        };
        self.client = Some(client);

        let loaded = self.call_within(
            "wait_loaded",
            json!({"timeout_ms": LOAD_WAIT.as_millis() as u64}),
            LOAD_WAIT + Duration::from_secs(5),
        )?;
        if let Some(mode) = arguments.get("mode").and_then(Value::as_str) {
            self.call("set_mode", json!({"mode": mode}))?;
        }
        // One row per list: the summary reads scalars, and a full dump of a
        // real vault is thousands of lines the model did not ask for.
        let state = self.call("state", json!({"rows": 1}))?;
        Ok(summary(&state, &loaded, pid, &self.config.socket))
    }

    /// Ask the app to exit, then make sure it did and clean up after it.
    pub fn quit(&mut self) -> Result<String, String> {
        if self.child.is_none() && self.client.is_none() && !self.config.socket.exists() {
            return Err(format!(
                "no silica is running: nothing listens on {}",
                self.config.socket.display()
            ));
        }
        let asked = self.call("quit", json!({}));
        let reaped = self.reap(QUIT_GRACE);
        // The app removes its own socket file on a clean exit; a killed one
        // leaves it behind, and a stale file makes the next launch look alive.
        // An app that is still running keeps its name: unlinking that would
        // leave a live window nothing can reach again.
        if asked.is_ok() || reaped.is_some() {
            let _ = std::fs::remove_file(&self.config.socket);
        }
        match (asked, reaped) {
            (Ok(_), Some(status)) => Ok(format!("silica quit ({status})")),
            (Ok(_), None) => Ok("silica quit".to_string()),
            (Err(e), Some(status)) => Ok(format!("silica exited {status} (`quit` said: {e})")),
            (Err(e), None) => Err(format!("could not quit silica: {e}")),
        }
    }

    /// One control call, with the configured timeout.
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.call_within(method, params, self.config.timeout)
    }

    fn call_within(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.connect()?;
        let answer = match self.client.as_mut() {
            Some(client) => client.call_timeout(method, params, timeout),
            None => return Err("no control connection".to_string()),
        };
        match answer {
            Ok(result) => Ok(result),
            // A child that died mid-call explains the transport error better
            // than the transport error does.
            Err(e) => match self.exited() {
                Some(status) => {
                    self.reap(Duration::ZERO);
                    Err(format!(
                        "silica exited ({status}) during `{method}`; call silica_launch again"
                    ))
                }
                // A client that is out of step cannot be reused: drop it so
                // the next call reconnects instead of reading a stale reply.
                None => {
                    if self.client.as_ref().is_some_and(Client::is_broken) {
                        self.client = None;
                    }
                    Err(e)
                }
            },
        }
    }

    /// Activate the window, let it settle, and capture it.
    pub fn screenshot(&mut self, crop: Option<&str>) -> Result<Vec<u8>, String> {
        let command = self.capture_command(crop)?;
        self.call("window", json!({"activate": true}))?;
        self.call("wait_idle", json!({}))?;
        let path = runtime_dir().join(format!("silica-mcp-{}.png", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let command = command.replace("{path}", &path.display().to_string());
        let output = shell_command(&command)
            .current_dir(&self.config.repo)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run the screenshot command: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "screenshot command failed ({}): {command}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let png = std::fs::read(&path)
            .map_err(|e| format!("{command} wrote no file at {}: {e}", path.display()))?;
        let _ = std::fs::remove_file(&path);
        if !png.starts_with(b"\x89PNG") {
            return Err(format!(
                "{command} wrote {} bytes that are not a PNG",
                png.len()
            ));
        }
        Ok(png)
    }

    /// The capture command with `{crop}` resolved, before anything is run: a
    /// crop this server cannot honour must fail before the window is poked.
    fn capture_command(&self, crop: Option<&str>) -> Result<String, String> {
        let template = &self.config.screenshot;
        match (crop, template.contains("{crop}")) {
            (None, false) => Ok(template.clone()),
            (None, true) => Err(
                "SILICA_MCP_SCREENSHOT has a `{crop}` placeholder, so `crop` is required"
                    .to_string(),
            ),
            (Some(_), false) => Err(format!(
                "this server cannot crop on its own: set SILICA_MCP_SCREENSHOT to a command \
                 with a `{{crop}}` placeholder, such as `{template} && magick {{path}} \
                 -crop {{crop}} {{path}}`"
            )),
            (Some(crop), true) => {
                // The template is run through `sh -c`, so a crop that is not a
                // geometry is a shell injection, not a mistake.
                if crop.is_empty() || !crop.bytes().all(is_geometry) {
                    return Err(format!(
                        "`crop` must be an ImageMagick geometry like `800x600+0+0`, not {crop:?}"
                    ));
                }
                Ok(template.replace("{crop}", crop))
            }
        }
    }

    fn spawn(&self, shell: bool, argv: &[String]) -> Result<Child, String> {
        let mut command = if shell {
            let quoted: Vec<String> = argv.iter().map(|a| quote(a)).collect();
            let line = if self.config.bin.contains("{args}") {
                self.config.bin.replace("{args}", &quoted.join(" "))
            } else {
                format!("{} {}", self.config.bin, quoted.join(" "))
            };
            shell_command(&exec(&line))
        } else {
            let mut command = Command::new(&self.config.bin);
            command.args(argv);
            command
        };
        command
            .current_dir(&self.config.repo)
            .stdin(Stdio::null())
            // This server's stdout carries the protocol: anything the app
            // prints there would be read as a JSON-RPC message.
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| format!("could not run {}: {e}", self.config.bin))
    }

    /// Connect lazily, so the thin wrappers also work against an app the user
    /// started themselves with `--control`.
    fn connect(&mut self) -> Result<(), String> {
        if let Some(status) = self.exited() {
            self.reap(Duration::ZERO);
            return Err(format!(
                "silica exited ({status}); call silica_launch to start it again"
            ));
        }
        if self.client.is_some() {
            return Ok(());
        }
        let socket = self.config.socket.display().to_string();
        let client =
            Client::connect(&self.config.socket, ATTACH_WAIT).map_err(|e| match e.kind() {
                ErrorKind::NotFound | ErrorKind::ConnectionRefused => format!(
                    "no silica is running: nothing listens on {socket}. \
                     Call silica_launch, or start the app yourself with `--control {socket}`."
                ),
                _ => format!("could not connect to {socket}: {e}"),
            })?;
        self.client = Some(client);
        Ok(())
    }

    /// The pid of a child that is still alive, reaping a dead one on the way.
    fn running_pid(&mut self) -> Option<u32> {
        if self.exited().is_some() {
            self.reap(Duration::ZERO);
        }
        self.child.as_ref().map(Child::id)
    }

    /// The exit status of a child that has already finished, if any.
    fn exited(&mut self) -> Option<ExitStatus> {
        let child = self.child.as_mut()?;
        child.try_wait().ok().flatten()
    }

    /// Drop the connection and collect the child, giving it `grace` to exit on
    /// its own before killing it.
    fn reap(&mut self, grace: Duration) -> Option<ExitStatus> {
        self.client = None;
        let mut child = self.child.take()?;
        let deadline = Instant::now() + grace;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                _ => break,
            }
        }
        let _ = child.kill();
        child.wait().ok()
    }
}

impl Drop for App {
    /// A child that outlived this server would hold the socket, the vault and
    /// a GPU context with nobody driving it, so stdin closing takes the app
    /// with it.
    fn drop(&mut self) {
        if self.child.is_none() {
            return;
        }
        let _ = self.call("quit", json!({}));
        self.reap(QUIT_GRACE);
        let _ = std::fs::remove_file(&self.config.socket);
    }
}

/// The platform's shell, given one command line: `sh -c` where there is one,
/// Windows PowerShell on Windows.
fn shell_command(line: &str) -> Command {
    #[cfg(unix)]
    {
        let mut command = Command::new("sh");
        command.arg("-c").arg(line);
        command
    }
    #[cfg(windows)]
    {
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", line]);
        command
    }
}

/// `line` as the shell's last act. `exec` keeps the pid we hold pointing at
/// the process to kill, instead of a shell that has forgotten about it.
/// PowerShell has no exec: there the pid is the shell's, which waits for the
/// app and ends with it, and a `kill` ends the shell alone — which is why
/// `quit` goes over the socket first, and why a plain path is the better
/// `SILICA_MCP_BIN` on Windows.
fn exec(line: &str) -> String {
    if cfg!(unix) {
        format!("exec {line}")
    } else {
        line.to_string()
    }
}

/// Quote one argument for the shell form of `SILICA_MCP_BIN`: `sh`'s single
/// quotes, or PowerShell's, which double a quote rather than escape it.
fn quote(argument: &str) -> String {
    if cfg!(unix) {
        format!("'{}'", argument.replace('\'', r#"'\''"#))
    } else {
        format!("'{}'", argument.replace('\'', "''"))
    }
}

/// A canonical path the way a user would write it. On Windows `canonicalize`
/// answers in the verbatim form, `\\?\C:\...`, which the app would show in
/// its title and write into its vault registry as a folder other than the
/// same one opened by hand; the prefix goes, for a drive path and a UNC one.
fn plain(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            if rest.as_bytes().get(1) == Some(&b':') {
                return PathBuf::from(rest);
            }
        }
    }
    path
}

/// ImageMagick geometry: digits, `x`, offsets, and the `%`/`!` modifiers.
fn is_geometry(byte: u8) -> bool {
    byte.is_ascii_digit() || b"x+-%!".contains(&byte)
}

fn number(arguments: &Value, name: &str) -> Option<f64> {
    arguments.get(name).and_then(Value::as_f64)
}

/// What came up, in four lines. Every key of the state dump may be null, so
/// nothing here insists on finding anything.
fn summary(state: &Value, loaded: &Value, pid: u32, socket: &Path) -> String {
    let vault = &state["vault"];
    let file = &state["file"];
    let mut out = format!("silica is running (pid {pid}) on {}\n", socket.display());
    out.push_str(&format!(
        "vault: {} at {} ({})\n",
        text_of(&vault["name"]).unwrap_or("?"),
        text_of(&vault["root"]).unwrap_or("?"),
        if vault["loaded"].as_bool() == Some(true) {
            "loaded"
        } else {
            "still loading"
        }
    ));
    out.push_str(&match text_of(&file["path"]) {
        Some(path) => format!(
            "file: {path} in {} mode{}\n",
            text_of(&file["mode"]).unwrap_or("?"),
            if file["dirty"].as_bool() == Some(true) {
                ", unsaved"
            } else {
                ""
            }
        ),
        None => "file: none open\n".to_string(),
    });
    out.push_str(&format!(
        "focus: {}, window {}x{}\n",
        text_of(&state["focus"]["name"]).unwrap_or("?"),
        state["window"]["width"],
        state["window"]["height"]
    ));
    if loaded["timed_out"].as_bool() == Some(true) {
        out.push_str("warning: the vault was still indexing when the wait timed out\n");
    }
    out.push_str("Read silica_state for everything else.");
    out
}

fn text_of(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use silica_control::net::UnixListener;
    use std::io::{BufRead, BufReader, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// An app that answers at once, except on the methods it was told to be
    /// slow about — those outlive any timeout here. It counts the connections
    /// it is given, which is how a reconnect shows up.
    struct Slow {
        path: PathBuf,
        connections: Arc<AtomicUsize>,
    }

    impl Slow {
        fn start(name: &str, slow: &'static [&'static str]) -> Slow {
            let path = std::env::temp_dir()
                .join(format!("silica-mcp-app-{}-{name}.sock", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let listener = UnixListener::bind(&path).expect("bind the fake socket");
            let connections = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&connections);
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    counted.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        let reader = BufReader::new(match stream.try_clone() {
                            Ok(stream) => stream,
                            Err(_) => return,
                        });
                        let mut writer = stream;
                        for line in reader.lines().map_while(Result::ok) {
                            let Ok(request) = serde_json::from_str::<Value>(&line) else {
                                continue;
                            };
                            let id = request["id"].clone();
                            let method = request["method"].as_str().unwrap_or_default();
                            if slow.contains(&method) {
                                std::thread::sleep(Duration::from_millis(400));
                            }
                            if writeln!(writer, "{}", json!({"id": id, "result": {"ok": true}}))
                                .is_err()
                            {
                                return;
                            }
                        }
                    });
                }
            });
            Slow { path, connections }
        }

        fn app(&self) -> App {
            App::new(Config {
                // Short enough that `hang` times out while its reply is still
                // in flight, which is the whole point of these tests.
                timeout: Duration::from_millis(60),
                ..Config::for_test(self.path.clone())
            })
        }
    }

    impl Drop for Slow {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    #[cfg(unix)]
    #[test]
    fn only_a_directory_of_ours_is_private_enough_for_the_socket() {
        let uid = our_uid();
        assert!(uid.is_some(), "every Unix can name our uid");
        let base = std::env::temp_dir().join(format!("silica-mcp-priv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).expect("a directory to test in");

        let private = base.join("private");
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(&private).expect("creates 0700");
        assert!(is_ours(&private, uid));

        // A directory anyone can enter is a directory anyone can plant a
        // socket name in.
        let open = base.join("open");
        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o755);
        builder.create(&open).expect("creates 0755");
        assert!(!is_ours(&open, uid));

        // Not a directory at all, and a symlink pointing at a good one.
        let file = base.join("file");
        std::fs::write(&file, b"").expect("writes");
        assert!(!is_ours(&file, uid));
        let link = base.join("link");
        std::os::unix::fs::symlink(&private, &link).expect("links");
        assert!(!is_ours(&link, uid));

        // With no uid to compare against, nothing pre-existing qualifies.
        assert!(!is_ours(&private, None));

        std::fs::remove_dir_all(&base).expect("cleans up");
    }

    #[cfg(windows)]
    #[test]
    fn a_verbatim_path_is_written_the_way_a_user_would() {
        assert_eq!(
            plain(PathBuf::from(r"\\?\C:\Users\me\vault")),
            PathBuf::from(r"C:\Users\me\vault")
        );
        assert_eq!(
            plain(PathBuf::from(r"\\?\UNC\nas\share\vault")),
            PathBuf::from(r"\\nas\share\vault")
        );
        // Anything else is left alone, including a device path.
        assert_eq!(
            plain(PathBuf::from(r"\\?\Volume{guid}\vault")),
            PathBuf::from(r"\\?\Volume{guid}\vault")
        );
        assert_eq!(
            plain(PathBuf::from(r"D:\vault")),
            PathBuf::from(r"D:\vault")
        );
    }

    #[test]
    fn a_timed_out_call_costs_only_its_own_connection() {
        let fake = Slow::start("desync", &["hang"]);
        let mut app = fake.app();

        let e = app.call("hang", json!({})).expect_err("times out");
        assert!(e.contains("reading hang"), "{e}");
        // The reply to `hang` is still coming, so that connection is out of
        // step for good; the next call has to get a new one.
        assert!(app.client.is_none(), "the broken client was kept");

        let reply = app.call("ping", json!({})).expect("a fresh connection");
        assert_eq!(reply, json!({"ok": true}));
        assert_eq!(fake.connections.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn quit_leaves_the_socket_of_an_app_that_did_not_answer() {
        // An app too busy to answer `quit` inside the timeout, and no child
        // of ours to reap: all this server knows is that it is still there.
        let fake = Slow::start("live-quit", &["quit"]);
        let mut app = fake.app();

        let e = app.quit().expect_err("nothing confirmed the exit");
        assert!(e.contains("could not quit"), "{e}");
        assert!(
            fake.path.exists(),
            "a live app's socket must stay reachable"
        );
    }
}
