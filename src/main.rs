use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use voice_runtime::app::RuntimeApp;
use voice_runtime::config::repos::load_repository_dir;
use voice_runtime::config::{
    load_selected_audio_device, load_selected_repositories, save_selected_audio_device,
};
use voice_runtime::frontend::gui::{run_gui, GuiConfig};
use voice_runtime::frontend::ipc::{
    default_socket_path, send_ipc_command, IpcCommand, IpcControlServer,
};
use voice_runtime::mode::RuntimeMode;
use voice_runtime::runtime::command_inventory::CommandInventory;
use voice_runtime::runtime::python_host::PythonCompatibilityHost;
use voice_runtime::runtime::talon_ast::TalonFile;
use voice_runtime::speech::{
    discover_default_model_path, enumerate_input_devices, load_wav_16khz_mono,
    phrase_inventory_for_mode_with_host, resolve_input_device_selector, AudioDeviceInfo,
    TranscribeCppEngine, VadConfig,
};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        println!("voice-runtime");
        return ExitCode::SUCCESS;
    }

    match args[0].as_str() {
        "-h" | "--help" | "help" => {
            print_usage();
            ExitCode::SUCCESS
        }
        "gui" => run_gui_cmd(&args[1..]),
        "devices" => run_devices(),
        "select-device" => run_select_device(&args[1..]),
        "listen" => run_listen(&args[1..]),
        "toggle" => run_ipc_client(IpcCommand::ToggleListening),
        "status" => run_ipc_client(IpcCommand::Status),
        "mode" => run_mode_cmd(&args[1..]),
        "simulate" => run_simulate(&args[1..]),
        "check-repo" => run_check_repo(&args[1..]),
        other => {
            eprintln!("unknown subcommand `{other}`");
            print_usage();
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    println!(
        "\
voice-runtime

USAGE:
    voice-runtime
    voice-runtime gui [--model <path.gguf>] [--device <index-or-name>] [--mode <global|code|browser|terminal>] [--repo <path>]...
    voice-runtime devices
    voice-runtime select-device [<index-or-name> | --clear]
    voice-runtime listen [--model <path.gguf>] [--device <index-or-name>] [--select-device] [--mode <global|code|browser|terminal>] [--repo <path>]... [--wav <path.wav>] [--once]
    voice-runtime toggle
    voice-runtime mode <cycle|auto|global|code|browser|terminal>
    voice-runtime status
    voice-runtime simulate [--mode <global|code|browser|terminal>] [--repo <path>]... <phrase>
    voice-runtime check-repo <path>..."
    );
}

fn parse_mode_arg(s: &str) -> Result<RuntimeMode, String> {
    match s.to_ascii_lowercase().as_str() {
        "global" => Ok(RuntimeMode::Global),
        "code" => Ok(RuntimeMode::Code),
        "browser" => Ok(RuntimeMode::Browser),
        "terminal" => Ok(RuntimeMode::Terminal),
        _ => Err(format!(
            "invalid mode `{s}` (expected global, code, browser, or terminal)"
        )),
    }
}

struct CommonListenArgs {
    model_path: Option<PathBuf>,
    device_selector: Option<String>,
    select_device_interactive: bool,
    mode: RuntimeMode,
    repos: Vec<PathBuf>,
    wav_path: Option<PathBuf>,
    once: bool,
}

fn parse_common_listen_args(args: &[String], allow_wav_once: bool) -> Result<CommonListenArgs, String> {
    let mut out = CommonListenArgs {
        model_path: None,
        device_selector: None,
        select_device_interactive: false,
        mode: RuntimeMode::Browser,
        repos: Vec::new(),
        wav_path: None,
        once: false,
    };

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--model" => {
                i += 1;
                let val = args.get(i).ok_or("--model requires a path")?;
                out.model_path = Some(PathBuf::from(val));
            }
            "--device" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or("--device requires a device index or name")?;
                out.device_selector = Some(val.clone());
            }
            "--select-device" if allow_wav_once => {
                out.select_device_interactive = true;
            }
            "--mode" => {
                i += 1;
                let val = args.get(i).ok_or("--mode requires a value")?;
                out.mode = parse_mode_arg(val)?;
            }
            "--repo" => {
                i += 1;
                let val = args.get(i).ok_or("--repo requires a path")?;
                out.repos.push(PathBuf::from(val));
            }
            "--wav" if allow_wav_once => {
                i += 1;
                let val = args.get(i).ok_or("--wav requires a path")?;
                out.wav_path = Some(PathBuf::from(val));
            }
            "--once" if allow_wav_once => {
                out.once = true;
            }
            unknown => return Err(format!("unknown option `{unknown}`")),
        }
        i += 1;
    }

    Ok(out)
}

fn build_runtime_app(mode: RuntimeMode, repos: &[PathBuf]) -> Result<RuntimeApp, String> {
    let effective_repos: Vec<PathBuf> = if repos.is_empty() {
        load_selected_repositories(None)
            .into_iter()
            .filter(|p| p.exists())
            .collect()
    } else {
        repos.to_vec()
    };

    RuntimeApp::with_default_backends(mode, &effective_repos)
        .map_err(|err| format!("failed to load repositories: {err}"))
}

fn print_audio_devices(devices: &[AudioDeviceInfo], saved_selection: Option<&str>) {
    if devices.is_empty() {
        println!("no audio input devices found");
        return;
    }

    for dev in devices {
        let mut tags = Vec::new();
        if dev.is_default {
            tags.push("default");
        }
        let is_active = match saved_selection {
            Some(sel) => sel == dev.name,
            None => dev.is_default,
        };
        if is_active {
            tags.push("active");
        }

        let format_info = match (dev.sample_rate, dev.channels) {
            (Some(rate), Some(ch)) => format!(" ({rate} Hz, {ch} ch)"),
            _ => String::new(),
        };
        let tag_str = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        println!("{}: {}{}{}", dev.index, dev.name, format_info, tag_str);
    }
}

fn prompt_select_audio_device(devices: &[AudioDeviceInfo]) -> Result<Option<AudioDeviceInfo>, String> {
    if devices.is_empty() {
        return Err("no audio input devices found".into());
    }

    let saved = load_selected_audio_device(None);
    print_audio_devices(devices, saved.as_deref());
    print!(
        "Select audio input device [1-{}, name/substring, or Enter for system default]: ",
        devices.len()
    );
    io::stdout()
        .flush()
        .map_err(|e| format!("failed to flush stdout: {e}"))?;

    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|e| format!("failed to read stdin: {e}"))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    resolve_input_device_selector(trimmed, devices)
        .map(Some)
        .map_err(|e| e.to_string())
}

fn run_gui_cmd(args: &[String]) -> ExitCode {
    let parsed = match parse_common_listen_args(args, false) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let cfg = GuiConfig {
        model_path: parsed.model_path,
        initial_mode: parsed.mode,
        repos: parsed.repos,
        device_selector: parsed.device_selector,
        socket_path: default_socket_path(),
    };

    if let Err(msg) = run_gui(cfg) {
        eprintln!("error: {msg}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run_ipc_client(cmd: IpcCommand) -> ExitCode {
    let socket = default_socket_path();
    match send_ipc_command(&socket, &cmd) {
        Ok(resp) => {
            println!("{resp}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!(
                "error: could not connect to running voice-runtime at `{}` ({err}). Start `voice-runtime gui` or `voice-runtime listen` first.",
                socket.display()
            );
            ExitCode::FAILURE
        }
    }
}

fn run_mode_cmd(args: &[String]) -> ExitCode {
    let Some(arg) = args.first() else {
        eprintln!("mode requires one of: cycle, auto, global, code, browser, terminal");
        return ExitCode::FAILURE;
    };
    let cmd = match arg.to_ascii_lowercase().as_str() {
        "cycle" => IpcCommand::CycleMode,
        "auto" => IpcCommand::ClearManualMode,
        other => match parse_mode_arg(other) {
            Ok(m) => IpcCommand::SetMode(m),
            Err(msg) => {
                eprintln!("{msg}");
                return ExitCode::FAILURE;
            }
        },
    };
    run_ipc_client(cmd)
}

fn run_devices() -> ExitCode {
    match enumerate_input_devices() {
        Ok(devices) => {
            let saved = load_selected_audio_device(None);
            print_audio_devices(&devices, saved.as_deref());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_select_device(args: &[String]) -> ExitCode {
    if args.len() == 1 && args[0] == "--clear" {
        return match save_selected_audio_device(None, None) {
            Ok(_) => {
                println!("cleared selected audio device (using system default)");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: failed to clear device selection: {err}");
                ExitCode::FAILURE
            }
        };
    }

    let devices = match enumerate_input_devices() {
        Ok(d) => d,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let chosen = if args.is_empty() {
        match prompt_select_audio_device(&devices) {
            Ok(opt) => opt,
            Err(msg) => {
                eprintln!("error: {msg}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        let selector = args.join(" ");
        match resolve_input_device_selector(&selector, &devices) {
            Ok(info) => Some(info),
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::FAILURE;
            }
        }
    };

    match chosen {
        Some(info) => match save_selected_audio_device(Some(&info.name), None) {
            Ok(_) => {
                println!("selected audio device {}: {}", info.index, info.name);
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: failed to save device selection: {err}");
                ExitCode::FAILURE
            }
        },
        None => match save_selected_audio_device(None, None) {
            Ok(_) => {
                println!("cleared selected audio device (using system default)");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error: failed to save device selection: {err}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run_listen(args: &[String]) -> ExitCode {
    let parsed = match parse_common_listen_args(args, true) {
        Ok(p) => p,
        Err(msg) => {
            eprintln!("{msg}");
            print_usage();
            return ExitCode::FAILURE;
        }
    };

    let resolved_model = match parsed.model_path.or_else(discover_default_model_path) {
        Some(path) => path,
        None => {
            eprintln!(
                "no local .gguf speech model found. Pass `--model <path.gguf>` or set `VOICE_RUNTIME_MODEL`."
            );
            return ExitCode::FAILURE;
        }
    };

    let mut engine = match TranscribeCppEngine::load(&resolved_model) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };

    let mut app = match build_runtime_app(parsed.mode, &parsed.repos) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };

    let mut once = parsed.once;
    if let Some(wav) = parsed.wav_path {
        let pcm = match load_wav_16khz_mono(&wav) {
            Ok(p) => p,
            Err(err) => {
                eprintln!("error: {err}");
                return ExitCode::FAILURE;
            }
        };
        engine.enqueue_pcm_16khz(pcm);
        once = true;
    } else {
        let effective_selector = if parsed.select_device_interactive {
            let devices = match enumerate_input_devices() {
                Ok(d) => d,
                Err(err) => {
                    eprintln!("error: {err}");
                    return ExitCode::FAILURE;
                }
            };
            match prompt_select_audio_device(&devices) {
                Ok(Some(info)) => {
                    let _ = save_selected_audio_device(Some(&info.name), None);
                    Some(info.name)
                }
                Ok(None) => {
                    let _ = save_selected_audio_device(None, None);
                    None
                }
                Err(msg) => {
                    eprintln!("error: {msg}");
                    return ExitCode::FAILURE;
                }
            }
        } else {
            parsed
                .device_selector
                .or_else(|| load_selected_audio_device(None))
        };

        if let Err(err) =
            engine.with_microphone(effective_selector.as_deref(), VadConfig::default())
        {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
        if let Some(active_name) = engine.microphone_device_name() {
            app.set_audio_device(active_name);
        }

        println!(
            "listening on `{}` (mode={}, model={}, arch={}) — speak a command (Ctrl+C to stop)",
            app.active_audio_device().unwrap_or("default"),
            app.active_mode(),
            resolved_model.display(),
            engine.model_arch(),
        );
    }

    if once {
        return match app.recognize_and_execute_once(&mut engine) {
            Ok(Some(transcript)) => {
                print_recognized_line(&app, &transcript);
                ExitCode::SUCCESS
            }
            Ok(None) => {
                println!("(no speech recognized)");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error (mode={}): {err}", app.active_mode());
                ExitCode::FAILURE
            }
        };
    }

    let ipc_server = match IpcControlServer::bind(&default_socket_path()) {
        Ok(srv) => Some(srv),
        Err(err) => {
            eprintln!("warning: could not bind IPC shortcut socket: {err}");
            None
        }
    };

    loop {
        if let Some(srv) = &ipc_server {
            for (cmd, resp) in srv.poll_once(&mut app) {
                println!("ipc ({cmd:?}): {resp}");
                if let IpcCommand::SelectDevice(_) = cmd {
                    let sel = app.active_audio_device().map(str::to_string);
                    if let Err(err) = engine.with_microphone(sel.as_deref(), VadConfig::default()) {
                        eprintln!("error switching microphone: {err}");
                    }
                }
            }
        }

        if !app.is_listening() {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }

        let phrases = phrase_inventory_for_mode_with_host(
            app.inventory(),
            app.python_host(),
            app.active_mode(),
        );
        match engine.poll_recognize_command(&phrases, Duration::from_millis(100)) {
            Ok(Some(cands)) if !cands.is_empty() => match app.execute_candidates(&cands) {
                Ok(Some(transcript)) => print_recognized_line(&app, &transcript),
                Ok(None) => {}
                Err(err) => eprintln!("error (mode={}): {err}", app.active_mode()),
            },
            Ok(_) => {}
            Err(err) => {
                eprintln!("error (mode={}): {err}", app.active_mode());
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
}

fn print_recognized_line(app: &RuntimeApp, transcript: &str) {
    let matched = app.last_matched_command_for_test().unwrap_or("-");
    let backend = app.last_backend_invocation_for_test();
    let backend_label = if backend.is_empty() {
        "builtin/mode"
    } else {
        backend
    };
    println!(
        "heard: \"{transcript}\" -> matched: {matched} (mode={}, backend={backend_label})",
        app.active_mode()
    );
}

fn run_simulate(args: &[String]) -> ExitCode {
    let mut mode = RuntimeMode::Global;
    let mut repos: Vec<PathBuf> = Vec::new();
    let mut phrase_parts: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("--mode requires a value");
                    return ExitCode::FAILURE;
                };
                match parse_mode_arg(val) {
                    Ok(m) => mode = m,
                    Err(msg) => {
                        eprintln!("{msg}");
                        return ExitCode::FAILURE;
                    }
                }
            }
            "--repo" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("--repo requires a path");
                    return ExitCode::FAILURE;
                };
                repos.push(PathBuf::from(val));
            }
            part => phrase_parts.push(part.to_string()),
        }
        i += 1;
    }

    let phrase = phrase_parts.join(" ");
    if phrase.trim().is_empty() {
        eprintln!("simulate requires a spoken phrase");
        print_usage();
        return ExitCode::FAILURE;
    }

    let mut app = match build_runtime_app(mode, &repos) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            return ExitCode::FAILURE;
        }
    };

    match app.handle_recognized_phrase(&phrase) {
        Ok(()) => {
            println!("mode: {}", app.active_mode());
            if let Some(cmd) = app.last_matched_command_for_test() {
                println!("matched: {cmd}");
            }
            let backend = app.last_backend_invocation_for_test();
            if !backend.is_empty() {
                println!("backend: {backend}");
            } else {
                println!("backend: builtin/python");
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error (mode={}): {err}", app.active_mode());
            ExitCode::FAILURE
        }
    }
}

fn run_check_repo(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("check-repo requires at least one path");
        print_usage();
        return ExitCode::FAILURE;
    }

    let mut files: Vec<TalonFile> = Vec::new();
    let mut python_host = PythonCompatibilityHost::default();
    let mut diagnostics: Vec<String> = Vec::new();

    for path_str in args {
        let path = PathBuf::from(path_str);
        if let Err(err) =
            load_repository_dir(&path, &mut files, &mut python_host, &mut diagnostics)
        {
            eprintln!("failed to check `{}`: {err}", path.display());
            return ExitCode::FAILURE;
        }
    }

    let inventory = CommandInventory::from_files(files);
    println!("files loaded: {}", inventory.files().len());
    for mode in [
        RuntimeMode::Global,
        RuntimeMode::Code,
        RuntimeMode::Browser,
        RuntimeMode::Terminal,
    ] {
        println!(
            "commands[{mode}]: {}",
            inventory.commands_for_mode(mode).len()
        );
    }
    println!("diagnostics: {}", diagnostics.len());
    for diag in diagnostics.iter().take(20) {
        println!("  - {diag}");
    }
    if diagnostics.len() > 20 {
        println!("  ... ({} more)", diagnostics.len() - 20);
    }

    ExitCode::SUCCESS
}
