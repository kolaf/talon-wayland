use crate::app::RuntimeApp;
use crate::mode::RuntimeMode;
use crate::speech::{enumerate_input_devices, resolve_input_device_selector};
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcCommand {
    ToggleListening,
    SetListening(bool),
    CycleMode,
    SetMode(RuntimeMode),
    ClearManualMode,
    SelectDevice(String),
    Status,
}

/// Resolves the Unix domain socket path for external Wayland shortcut IPC control.
pub fn default_socket_path() -> PathBuf {
    if let Ok(custom) = std::env::var("VOICE_RUNTIME_SOCKET") {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        let trimmed = runtime_dir.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("voice-runtime.sock");
        }
    }
    PathBuf::from("/tmp/voice-runtime.sock")
}

pub fn format_ipc_command(cmd: &IpcCommand) -> String {
    match cmd {
        IpcCommand::ToggleListening => "TOGGLE".into(),
        IpcCommand::SetListening(true) => "LISTEN ON".into(),
        IpcCommand::SetListening(false) => "LISTEN OFF".into(),
        IpcCommand::CycleMode => "MODE cycle".into(),
        IpcCommand::ClearManualMode => "MODE auto".into(),
        IpcCommand::SetMode(mode) => format!("MODE {mode}"),
        IpcCommand::SelectDevice(sel) => format!("DEVICE {sel}"),
        IpcCommand::Status => "STATUS".into(),
    }
}

pub fn parse_ipc_command(line: &str) -> Result<IpcCommand, String> {
    let trimmed = line.trim();
    if trimmed.eq_ignore_ascii_case("TOGGLE") {
        return Ok(IpcCommand::ToggleListening);
    }
    if trimmed.eq_ignore_ascii_case("STATUS") {
        return Ok(IpcCommand::Status);
    }
    if trimmed.eq_ignore_ascii_case("LISTEN ON") {
        return Ok(IpcCommand::SetListening(true));
    }
    if trimmed.eq_ignore_ascii_case("LISTEN OFF") {
        return Ok(IpcCommand::SetListening(false));
    }
    if let Some(rest) = trimmed
        .strip_prefix("MODE ")
        .or_else(|| trimmed.strip_prefix("mode "))
    {
        let arg = rest.trim().to_ascii_lowercase();
        return match arg.as_str() {
            "cycle" => Ok(IpcCommand::CycleMode),
            "auto" => Ok(IpcCommand::ClearManualMode),
            "global" => Ok(IpcCommand::SetMode(RuntimeMode::Global)),
            "code" => Ok(IpcCommand::SetMode(RuntimeMode::Code)),
            "browser" => Ok(IpcCommand::SetMode(RuntimeMode::Browser)),
            "terminal" => Ok(IpcCommand::SetMode(RuntimeMode::Terminal)),
            _ => Err(format!("unknown mode `{arg}`")),
        };
    }
    if let Some(rest) = trimmed
        .strip_prefix("DEVICE ")
        .or_else(|| trimmed.strip_prefix("device "))
    {
        let sel = rest.trim();
        if sel.is_empty() {
            return Err("DEVICE requires a selector".into());
        }
        return Ok(IpcCommand::SelectDevice(sel.to_string()));
    }

    Err(format!("unknown IPC command `{trimmed}`"))
}

pub fn apply_ipc_command(app: &mut RuntimeApp, cmd: &IpcCommand) -> String {
    match cmd {
        IpcCommand::ToggleListening => {
            app.toggle_listening();
        }
        IpcCommand::SetListening(enabled) => {
            app.set_listening(*enabled);
        }
        IpcCommand::CycleMode => {
            app.cycle_mode();
        }
        IpcCommand::SetMode(mode) => {
            app.set_manual_mode(*mode);
        }
        IpcCommand::ClearManualMode => {
            app.clear_manual_mode();
        }
        IpcCommand::SelectDevice(selector) => {
            if selector.eq_ignore_ascii_case("default") || selector == "--clear" {
                app.clear_audio_device();
            } else {
                let resolved = enumerate_input_devices()
                    .ok()
                    .and_then(|devs| resolve_input_device_selector(selector, &devs).ok())
                    .map(|info| info.name)
                    .unwrap_or_else(|| selector.clone());
                app.set_audio_device(resolved);
            }
        }
        IpcCommand::Status => {}
    }

    let st = app.status();
    format!(
        "listening={} mode={} source={:?} device={}",
        st.listening,
        st.mode,
        st.mode_source,
        st.audio_device.as_deref().unwrap_or("system-default")
    )
}

/// Non-blocking Unix domain socket server accepting Wayland shortcut commands.
pub struct IpcControlServer {
    socket_path: PathBuf,
    listener: UnixListener,
}

impl std::fmt::Debug for IpcControlServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IpcControlServer")
            .field("socket_path", &self.socket_path)
            .finish_non_exhaustive()
    }
}

impl Drop for IpcControlServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl IpcControlServer {
    pub fn bind(socket_path: &Path) -> io::Result<Self> {
        if let Some(parent) = socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if socket_path.exists() {
            let _ = std::fs::remove_file(socket_path);
        }
        let listener = UnixListener::bind(socket_path)?;
        listener.set_nonblocking(true)?;
        Ok(Self {
            socket_path: socket_path.to_path_buf(),
            listener,
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Accepts and executes any pending IPC commands without blocking.
    pub fn poll_once(&self, app: &mut RuntimeApp) -> Vec<(IpcCommand, String)> {
        let mut handled = Vec::new();
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                    let mut reader = BufReader::new(stream.try_clone().unwrap_or_else(|_| {
                        panic!("failed to clone accepted UnixStream")
                    }));
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_ok() && !line.trim().is_empty() {
                        match parse_ipc_command(&line) {
                            Ok(cmd) => {
                                let resp = apply_ipc_command(app, &cmd);
                                let _ = writeln!(stream, "{resp}");
                                let _ = stream.flush();
                                handled.push((cmd, resp));
                            }
                            Err(err) => {
                                let _ = writeln!(stream, "error: {err}");
                                let _ = stream.flush();
                            }
                        }
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        handled
    }
}

/// Sends a control command to a running `voice-runtime` GUI or `listen` instance.
pub fn send_ipc_command(socket_path: &Path, cmd: &IpcCommand) -> io::Result<String> {
    let mut stream = UnixStream::connect(socket_path)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;

    let payload = format!("{}\n", format_ipc_command(cmd));
    stream.write_all(payload.as_bytes())?;
    stream.flush()?;

    let mut reader = BufReader::new(stream);
    let mut resp = String::new();
    reader.read_line(&mut resp)?;
    Ok(resp.trim().to_string())
}
