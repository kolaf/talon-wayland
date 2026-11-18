use crate::app::RuntimeApp;
use crate::config::{
    load_selected_audio_device, load_selected_model, load_selected_repositories,
    save_selected_audio_device, save_selected_model, save_selected_repositories,
};
use crate::frontend::ipc::{IpcCommand, IpcControlServer};
use crate::mode::RuntimeMode;
use crate::speech::{
    discover_available_models, enumerate_input_devices, phrase_inventory_for_mode_with_host,
    AudioDeviceInfo, SpeechCandidate, TranscribeCppEngine, VadConfig,
};
use glib::ControlFlow;
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

const REPO_SLOT_NAMES: [&str; 3] = ["community", "cursorless-talon", "rango-talon"];

#[derive(Debug, Clone)]
pub struct GuiConfig {
    pub model_path: Option<PathBuf>,
    pub initial_mode: RuntimeMode,
    pub repos: Vec<PathBuf>,
    pub device_selector: Option<String>,
    pub socket_path: PathBuf,
}

enum WorkerEvent {
    Candidates(Vec<SpeechCandidate>),
    Status(String),
    Error(String),
}

struct GuiState {
    app: RuntimeApp,
    devices: Vec<AudioDeviceInfo>,
    models: Vec<PathBuf>,
    active_model: Option<PathBuf>,
    repo_slots: [Option<PathBuf>; 3],
    repo_status: String,
    suppress_combo: bool,
    last_heard: String,
    last_command: String,
}

/// Launches the native GTK3 Wayland GUI showing active mode, microphone selector,
/// speech model picker, Talon repository folder pickers, listening toggle, and Wayland IPC status.
pub fn run_gui(config: GuiConfig) -> Result<(), String> {
    gtk::init().map_err(|e| format!("failed to initialize GTK display: {e}"))?;

    let initial_repos = if config.repos.is_empty() {
        load_selected_repositories(None)
            .into_iter()
            .filter(|p| p.exists())
            .collect::<Vec<_>>()
    } else {
        config.repos.clone()
    };

    let repo_slots = assign_repo_slots(&initial_repos);
    let active_paths = collect_slot_paths(&repo_slots);
    let mut app = RuntimeApp::with_default_backends(config.initial_mode, &active_paths)
        .map_err(|e| format!("failed to load configured repositories: {e}"))?;

    let initial_repo_status = format_repo_status(&app);
    let devices = enumerate_input_devices().unwrap_or_default();

    let initial_device = config
        .device_selector
        .clone()
        .or_else(|| load_selected_audio_device(None));
    if let Some(sel) = initial_device {
        if let Ok(info) = app.select_audio_device(&sel, &devices) {
            let _ = save_selected_audio_device(Some(&info.name), None);
        } else {
            app.set_audio_device(sel);
        }
    } else if let Some(def_dev) = devices.iter().find(|d| d.is_default) {
        app.set_audio_device(def_dev.name.clone());
    }

    let mut models = discover_available_models();
    let active_model = config
        .model_path
        .clone()
        .or_else(|| load_selected_model(None).filter(|p| p.is_file()))
        .or_else(|| models.first().cloned());
    if let Some(m) = &active_model {
        if !models.contains(m) {
            models.insert(0, m.clone());
        }
    }

    let ipc_server = IpcControlServer::bind(&config.socket_path)
        .map_err(|e| format!("failed to bind IPC socket `{}`: {e}", config.socket_path.display()))?;

    let listening_flag = Arc::new(AtomicBool::new(app.is_listening()));
    let stop_flag = Arc::new(AtomicBool::new(false));
    let desired_device = Arc::new(Mutex::new(app.active_audio_device().map(str::to_string)));
    let device_dirty = Arc::new(AtomicBool::new(true));
    let desired_model = Arc::new(Mutex::new(active_model.clone()));
    let model_dirty = Arc::new(AtomicBool::new(active_model.is_some()));
    let active_phrases = Arc::new(Mutex::new(phrase_inventory_for_mode_with_host(
        app.inventory(),
        app.python_host(),
        app.active_mode(),
    )));

    let (tx, rx) = mpsc::channel::<WorkerEvent>();

    spawn_audio_worker(
        Arc::clone(&listening_flag),
        Arc::clone(&stop_flag),
        Arc::clone(&desired_model),
        Arc::clone(&model_dirty),
        Arc::clone(&desired_device),
        Arc::clone(&device_dirty),
        Arc::clone(&active_phrases),
        tx,
    );

    let state = Rc::new(RefCell::new(GuiState {
        app,
        devices,
        models,
        active_model,
        repo_slots,
        repo_status: initial_repo_status,
        suppress_combo: false,
        last_heard: "—".into(),
        last_command: "—".into(),
    }));

    // Build GTK3 window
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Voice Runtime — Wayland Command Control");
    window.set_default_size(640, 580);
    window.set_border_width(14);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);

    // 1. Listening toggle button
    let toggle_btn = gtk::Button::with_label("");
    toggle_btn.set_height_request(44);
    root.pack_start(&toggle_btn, false, false, 0);

    // 2. Microphone selector section
    let mic_frame = gtk::Frame::new(Some("Active Microphone"));
    let mic_box = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    mic_box.set_border_width(8);
    let mic_combo = gtk::ComboBoxText::new();
    let mic_refresh_btn = gtk::Button::with_label("Refresh");
    mic_box.pack_start(&mic_combo, true, true, 0);
    mic_box.pack_start(&mic_refresh_btn, false, false, 0);
    mic_frame.add(&mic_box);
    root.pack_start(&mic_frame, false, false, 0);

    // 3. Speech Model selector & file picker section
    let model_frame = gtk::Frame::new(Some("Speech Model (.gguf / .bin)"));
    let model_vbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
    model_vbox.set_border_width(8);
    let model_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let model_combo = gtk::ComboBoxText::new();
    let model_browse_btn = gtk::Button::with_label("Browse...");
    let model_refresh_btn = gtk::Button::with_label("Refresh");
    model_row.pack_start(&model_combo, true, true, 0);
    model_row.pack_start(&model_browse_btn, false, false, 0);
    model_row.pack_start(&model_refresh_btn, false, false, 0);
    let model_status_label = gtk::Label::new(None);
    model_status_label.set_xalign(0.0);
    model_status_label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
    model_vbox.pack_start(&model_row, false, false, 0);
    model_vbox.pack_start(&model_status_label, false, false, 0);
    model_frame.add(&model_vbox);
    root.pack_start(&model_frame, false, false, 0);

    // 4. Talon Repositories section with folder pickers
    let repo_frame = gtk::Frame::new(Some("Talon Repositories"));
    let repo_vbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
    repo_vbox.set_border_width(8);
    let mut repo_path_labels: Vec<gtk::Label> = Vec::with_capacity(3);
    let mut repo_browse_btns: Vec<gtk::Button> = Vec::with_capacity(3);
    let mut repo_clear_btns: Vec<gtk::Button> = Vec::with_capacity(3);

    for slot_name in REPO_SLOT_NAMES {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name_lbl = gtk::Label::new(Some(&format!("{slot_name}:")));
        name_lbl.set_width_chars(17);
        name_lbl.set_xalign(0.0);

        let path_lbl = gtk::Label::new(Some("(not selected)"));
        path_lbl.set_xalign(0.0);
        path_lbl.set_ellipsize(gtk::pango::EllipsizeMode::Start);

        let browse_btn = gtk::Button::with_label("Browse...");
        let clear_btn = gtk::Button::with_label("Clear");

        row.pack_start(&name_lbl, false, false, 0);
        row.pack_start(&path_lbl, true, true, 0);
        row.pack_start(&browse_btn, false, false, 0);
        row.pack_start(&clear_btn, false, false, 0);

        repo_vbox.pack_start(&row, false, false, 0);
        repo_path_labels.push(path_lbl);
        repo_browse_btns.push(browse_btn);
        repo_clear_btns.push(clear_btn);
    }

    let repo_summary_label = gtk::Label::new(None);
    repo_summary_label.set_xalign(0.0);
    repo_summary_label.set_line_wrap(true);
    repo_vbox.pack_start(&repo_summary_label, false, false, 2);
    repo_frame.add(&repo_vbox);
    root.pack_start(&repo_frame, false, false, 0);

    // 5. Mode section
    let mode_frame = gtk::Frame::new(Some("Active Workflow Mode"));
    let mode_vbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
    mode_vbox.set_border_width(8);
    let mode_label = gtk::Label::new(None);
    mode_label.set_xalign(0.0);
    let mode_btn_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let btn_global = gtk::Button::with_label("Global");
    let btn_code = gtk::Button::with_label("Code");
    let btn_browser = gtk::Button::with_label("Browser");
    let btn_terminal = gtk::Button::with_label("Terminal");
    let btn_cycle = gtk::Button::with_label("Cycle");
    let btn_auto = gtk::Button::with_label("Auto");
    for btn in [
        &btn_global,
        &btn_code,
        &btn_browser,
        &btn_terminal,
        &btn_cycle,
        &btn_auto,
    ] {
        mode_btn_row.pack_start(btn, true, true, 0);
    }
    mode_vbox.pack_start(&mode_label, false, false, 0);
    mode_vbox.pack_start(&mode_btn_row, false, false, 0);
    mode_frame.add(&mode_vbox);
    root.pack_start(&mode_frame, false, false, 0);

    // 6. Status / Activity section
    let status_frame = gtk::Frame::new(Some("Status & Shortcut IPC"));
    let status_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    status_box.set_border_width(8);
    let mic_status_label = gtk::Label::new(None);
    let heard_label = gtk::Label::new(None);
    let command_label = gtk::Label::new(None);
    let socket_text = format!(
        "Shortcut IPC: `voice-runtime toggle` / `voice-runtime mode cycle` ({})",
        config.socket_path.display()
    );
    let socket_label = gtk::Label::new(Some(&socket_text));
    for lbl in [
        &mic_status_label,
        &heard_label,
        &command_label,
        &socket_label,
    ] {
        lbl.set_xalign(0.0);
        lbl.set_line_wrap(true);
        status_box.pack_start(lbl, false, false, 0);
    }
    status_frame.add(&status_box);
    root.pack_start(&status_frame, true, true, 0);

    window.add(&root);

    let sync_ui = {
        let state = Rc::clone(&state);
        let toggle_btn = toggle_btn.clone();
        let mode_label = mode_label.clone();
        let mic_combo = mic_combo.clone();
        let model_combo = model_combo.clone();
        let model_status_label = model_status_label.clone();
        let mic_status_label = mic_status_label.clone();
        let heard_label = heard_label.clone();
        let command_label = command_label.clone();
        let repo_path_labels = repo_path_labels.clone();
        let repo_summary_label = repo_summary_label.clone();
        let listening_flag = Arc::clone(&listening_flag);
        let active_phrases = Arc::clone(&active_phrases);

        Rc::new(move |repopulate_combos: bool| {
            let mut st = state.borrow_mut();
            let status = st.app.status();

            listening_flag.store(status.listening, Ordering::Release);
            toggle_btn.set_label(if status.listening {
                "Listening: ON  (Click to Pause)"
            } else {
                "Listening: OFF  (Click to Resume)"
            });

            mode_label.set_text(&format!(
                "Mode: {}  ({:?})",
                status.mode, status.mode_source
            ));

            mic_status_label.set_text(&format!(
                "Microphone: {}",
                status.audio_device.as_deref().unwrap_or("system default")
            ));

            match &st.active_model {
                Some(path) => model_status_label.set_text(&format!("Path: {}", path.display())),
                None => model_status_label.set_text(
                    "No speech model selected (checked ~/.cache/huggingface/hub, Handy folders, ./models)",
                ),
            }

            for (idx, lbl) in repo_path_labels.iter().enumerate() {
                match &st.repo_slots[idx] {
                    Some(path) => lbl.set_text(&path.display().to_string()),
                    None => lbl.set_text("(not selected)"),
                }
            }
            repo_summary_label.set_text(&st.repo_status);

            heard_label.set_text(&format!("Last heard: {}", st.last_heard));
            command_label.set_text(&format!("Last command: {}", st.last_command));

            if let Ok(mut guard) = active_phrases.lock() {
                *guard = phrase_inventory_for_mode_with_host(
                    st.app.inventory(),
                    st.app.python_host(),
                    status.mode,
                );
            }

            if repopulate_combos {
                st.suppress_combo = true;

                mic_combo.remove_all();
                let mut selected_mic: Option<u32> = None;
                for (pos, dev) in st.devices.iter().enumerate() {
                    let suffix = if dev.is_default { " [default]" } else { "" };
                    mic_combo.append_text(&format!("{}: {}{}", dev.index, dev.name, suffix));
                    if status.audio_device.as_deref() == Some(dev.name.as_str()) {
                        selected_mic = Some(pos as u32);
                    }
                }
                if let Some(idx) = selected_mic {
                    mic_combo.set_active(Some(idx));
                } else if !st.devices.is_empty() {
                    mic_combo.set_active(Some(0));
                }

                model_combo.remove_all();
                let mut selected_model: Option<u32> = None;
                for (pos, path) in st.models.iter().enumerate() {
                    model_combo.append_text(&format_model_label(path));
                    if st.active_model.as_ref() == Some(path) {
                        selected_model = Some(pos as u32);
                    }
                }
                if let Some(idx) = selected_model {
                    model_combo.set_active(Some(idx));
                } else if !st.models.is_empty() {
                    model_combo.set_active(Some(0));
                }

                st.suppress_combo = false;
            }
        })
    };

    sync_ui(true);

    // Toggle listening button handler
    {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        toggle_btn.connect_clicked(move |_| {
            {
                let mut st = state.borrow_mut();
                st.app.toggle_listening();
            }
            sync_ui(false);
        });
    }

    // Speech Model combo selection handler
    {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        let desired_model = Arc::clone(&desired_model);
        let model_dirty = Arc::clone(&model_dirty);
        model_combo.connect_changed(move |combo| {
            let Some(active_idx) = combo.active() else {
                return;
            };
            let picked = {
                let mut st = state.borrow_mut();
                if st.suppress_combo {
                    return;
                }
                let Some(path) = st.models.get(active_idx as usize).cloned() else {
                    return;
                };
                st.active_model = Some(path.clone());
                let _ = save_selected_model(Some(&path), None);
                path
            };
            if let Ok(mut guard) = desired_model.lock() {
                *guard = Some(picked);
            }
            model_dirty.store(true, Ordering::Release);
            sync_ui(false);
        });
    }

    // Speech Model Browse... file picker handler
    {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        let parent_window = window.clone();
        let desired_model = Arc::clone(&desired_model);
        let model_dirty = Arc::clone(&model_dirty);
        model_browse_btn.connect_clicked(move |_| {
            let dialog = gtk::FileChooserDialog::with_buttons(
                Some("Select Speech Model (.gguf or .bin)"),
                Some(&parent_window),
                gtk::FileChooserAction::Open,
                &[
                    ("Cancel", gtk::ResponseType::Cancel),
                    ("Select Model", gtk::ResponseType::Accept),
                ],
            );

            let model_filter = gtk::FileFilter::new();
            model_filter.set_name(Some("Speech models (*.gguf, *.bin)"));
            model_filter.add_pattern("*.gguf");
            model_filter.add_pattern("*.bin");
            dialog.add_filter(model_filter);

            let all_filter = gtk::FileFilter::new();
            all_filter.set_name(Some("All files"));
            all_filter.add_pattern("*");
            dialog.add_filter(all_filter);

            if let Some(existing) = state.borrow().active_model.clone() {
                if let Some(parent) = existing.parent() {
                    if parent.exists() {
                        dialog.set_current_folder(parent);
                    }
                }
            }

            let response = dialog.run();
            let picked = if response == gtk::ResponseType::Accept {
                dialog.filename()
            } else {
                None
            };
            dialog.close();

            if let Some(model_path) = picked {
                {
                    let mut st = state.borrow_mut();
                    if !st.models.contains(&model_path) {
                        st.models.insert(0, model_path.clone());
                    }
                    st.active_model = Some(model_path.clone());
                    let _ = save_selected_model(Some(&model_path), None);
                }
                if let Ok(mut guard) = desired_model.lock() {
                    *guard = Some(model_path);
                }
                model_dirty.store(true, Ordering::Release);
                sync_ui(true);
            }
        });
    }

    // Refresh models button handler
    {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        model_refresh_btn.connect_clicked(move |_| {
            {
                let mut st = state.borrow_mut();
                let mut found = discover_available_models();
                if let Some(curr) = &st.active_model {
                    if curr.is_file() && !found.contains(curr) {
                        found.insert(0, curr.clone());
                    }
                }
                st.models = found;
            }
            sync_ui(true);
        });
    }

    // Repository folder picker & clear buttons
    for (slot_idx, slot_name) in REPO_SLOT_NAMES.iter().enumerate() {
        let browse_btn = &repo_browse_btns[slot_idx];
        let clear_btn = &repo_clear_btns[slot_idx];

        {
            let state = Rc::clone(&state);
            let sync_ui = Rc::clone(&sync_ui);
            let parent_window = window.clone();
            let slot_title = format!("Select {slot_name} Repository Folder");
            browse_btn.connect_clicked(move |_| {
                let dialog = gtk::FileChooserDialog::with_buttons(
                    Some(&slot_title),
                    Some(&parent_window),
                    gtk::FileChooserAction::SelectFolder,
                    &[
                        ("Cancel", gtk::ResponseType::Cancel),
                        ("Select Folder", gtk::ResponseType::Accept),
                    ],
                );
                if let Some(existing) = state.borrow().repo_slots[slot_idx].clone() {
                    if existing.exists() {
                        dialog.set_current_folder(existing);
                    }
                }
                let response = dialog.run();
                let picked = if response == gtk::ResponseType::Accept {
                    dialog.filename()
                } else {
                    None
                };
                dialog.close();

                if let Some(folder) = picked {
                    {
                        let mut st = state.borrow_mut();
                        st.repo_slots[slot_idx] = Some(folder);
                        apply_repo_slot_changes(&mut st);
                    }
                    sync_ui(false);
                }
            });
        }

        {
            let state = Rc::clone(&state);
            let sync_ui = Rc::clone(&sync_ui);
            clear_btn.connect_clicked(move |_| {
                {
                    let mut st = state.borrow_mut();
                    st.repo_slots[slot_idx] = None;
                    apply_repo_slot_changes(&mut st);
                }
                sync_ui(false);
            });
        }
    }

    // Microphone combo selection handler
    {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        let desired_device = Arc::clone(&desired_device);
        let device_dirty = Arc::clone(&device_dirty);
        mic_combo.connect_changed(move |combo| {
            let Some(active_idx) = combo.active() else {
                return;
            };
            let mut changed_name: Option<String> = None;
            {
                let mut st = state.borrow_mut();
                if st.suppress_combo {
                    return;
                }
                if let Some(dev) = st.devices.get(active_idx as usize).cloned() {
                    st.app.set_audio_device(dev.name.clone());
                    let _ = save_selected_audio_device(Some(&dev.name), None);
                    changed_name = Some(dev.name);
                }
            }
            if let Some(name) = changed_name {
                if let Ok(mut guard) = desired_device.lock() {
                    *guard = Some(name);
                }
                device_dirty.store(true, Ordering::Release);
                sync_ui(false);
            }
        });
    }

    // Refresh devices button handler
    {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        mic_refresh_btn.connect_clicked(move |_| {
            if let Ok(devs) = enumerate_input_devices() {
                let mut st = state.borrow_mut();
                st.devices = devs;
            }
            sync_ui(true);
        });
    }

    // Mode buttons
    for (btn, target_mode) in [
        (&btn_global, Some(RuntimeMode::Global)),
        (&btn_code, Some(RuntimeMode::Code)),
        (&btn_browser, Some(RuntimeMode::Browser)),
        (&btn_terminal, Some(RuntimeMode::Terminal)),
    ] {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        btn.connect_clicked(move |_| {
            if let Some(m) = target_mode {
                state.borrow_mut().app.set_manual_mode(m);
            }
            sync_ui(false);
        });
    }
    {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        btn_cycle.connect_clicked(move |_| {
            state.borrow_mut().app.cycle_mode();
            sync_ui(false);
        });
    }
    {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        btn_auto.connect_clicked(move |_| {
            state.borrow_mut().app.clear_manual_mode();
            sync_ui(false);
        });
    }

    // Window close
    {
        let stop_flag = Arc::clone(&stop_flag);
        window.connect_delete_event(move |_, _| {
            stop_flag.store(true, Ordering::Release);
            gtk::main_quit();
            glib::Propagation::Proceed
        });
    }

    // Poll IPC control socket + audio worker events at 20 Hz
    {
        let state = Rc::clone(&state);
        let sync_ui = Rc::clone(&sync_ui);
        let desired_device = Arc::clone(&desired_device);
        let device_dirty = Arc::clone(&device_dirty);
        glib::timeout_add_local(Duration::from_millis(50), move || {
            let mut ui_dirty = false;
            let mut combo_dirty = false;

            let handled = {
                let mut st = state.borrow_mut();
                ipc_server.poll_once(&mut st.app)
            };
            if !handled.is_empty() {
                ui_dirty = true;
                for (cmd, _resp) in handled {
                    if let IpcCommand::SelectDevice(_) = cmd {
                        let new_dev = state
                            .borrow()
                            .app
                            .active_audio_device()
                            .map(str::to_string);
                        let _ = save_selected_audio_device(new_dev.as_deref(), None);
                        if let Ok(mut guard) = desired_device.lock() {
                            *guard = new_dev;
                        }
                        device_dirty.store(true, Ordering::Release);
                        combo_dirty = true;
                    }
                }
            }

            while let Ok(evt) = rx.try_recv() {
                let mut st = state.borrow_mut();
                match evt {
                    WorkerEvent::Candidates(cands) => {
                        if !st.app.is_listening() || cands.is_empty() {
                            continue;
                        }
                        match st.app.execute_candidates(&cands) {
                            Ok(Some(heard)) => {
                                st.last_heard = heard;
                                let matched = st
                                    .app
                                    .last_matched_command_for_test()
                                    .unwrap_or("-")
                                    .to_string();
                                let backend = st.app.last_backend_invocation_for_test();
                                st.last_command = if backend.is_empty() {
                                    matched
                                } else {
                                    format!("{matched} ({backend})")
                                };
                            }
                            Ok(None) => {}
                            Err(err) => {
                                st.last_heard = cands
                                    .first()
                                    .map(|c| c.transcript.clone())
                                    .unwrap_or_default();
                                st.last_command = format!("error: {err}");
                            }
                        }
                        ui_dirty = true;
                    }
                    WorkerEvent::Status(msg) => {
                        st.last_command = msg;
                        ui_dirty = true;
                    }
                    WorkerEvent::Error(msg) => {
                        st.last_command = format!("audio error: {msg}");
                        ui_dirty = true;
                    }
                }
            }

            if ui_dirty {
                sync_ui(combo_dirty);
            }

            ControlFlow::Continue
        });
    }

    window.show_all();
    gtk::main();
    stop_flag.store(true, Ordering::Release);
    Ok(())
}

fn format_model_label(path: &Path) -> String {
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("model");
    let full = path.display().to_string();
    if full.contains("/huggingface/hub/") {
        format!("{file_name}  [hf-hub cache]")
    } else if full.contains("handy") {
        format!("{file_name}  [handy]")
    } else {
        format!("{file_name}  ({full})")
    }
}

fn assign_repo_slots(paths: &[PathBuf]) -> [Option<PathBuf>; 3] {
    let mut slots: [Option<PathBuf>; 3] = [None, None, None];
    let mut unmatched = Vec::new();

    for path in paths {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if name.contains("community") && slots[0].is_none() {
            slots[0] = Some(path.clone());
        } else if name.contains("cursorless") && slots[1].is_none() {
            slots[1] = Some(path.clone());
        } else if name.contains("rango") && slots[2].is_none() {
            slots[2] = Some(path.clone());
        } else {
            unmatched.push(path.clone());
        }
    }

    for path in unmatched {
        if let Some(empty) = slots.iter_mut().find(|s| s.is_none()) {
            *empty = Some(path);
        }
    }
    slots
}

fn collect_slot_paths(slots: &[Option<PathBuf>; 3]) -> Vec<PathBuf> {
    slots.iter().flatten().cloned().collect()
}

fn apply_repo_slot_changes(st: &mut GuiState) {
    let paths = collect_slot_paths(&st.repo_slots);
    let _ = save_selected_repositories(&paths, None);
    if paths.is_empty() {
        st.app.restore_fallback_commands();
        st.repo_status = format_repo_status(&st.app);
    } else {
        match st.app.reload_repositories(&paths) {
            Ok(summary) => {
                st.repo_status = format!(
                    "Loaded: {} repo(s), {} .talon file(s), {} command(s) ({} diagnostic(s))",
                    summary.repo_count,
                    summary.files_loaded,
                    summary.total_commands,
                    summary.diagnostics.len()
                );
            }
            Err(err) => {
                st.repo_status = format!("Repository load error: {err}");
            }
        }
    }
}

fn format_repo_status(app: &RuntimeApp) -> String {
    if app.configured_repos().is_empty() {
        "Using built-in sample commands (select repository folders above)".to_string()
    } else {
        let files = app.inventory().files();
        let total_cmds: usize = files.iter().map(|f| f.commands.len()).sum();
        let diags = app.inventory().diagnostics().len();
        format!(
            "Loaded: {} repo(s), {} .talon file(s), {} command(s) ({} diagnostic(s))",
            app.configured_repos().len(),
            files.len(),
            total_cmds,
            diags
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_audio_worker(
    listening_flag: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    desired_model: Arc<Mutex<Option<PathBuf>>>,
    model_dirty: Arc<AtomicBool>,
    desired_device: Arc<Mutex<Option<String>>>,
    device_dirty: Arc<AtomicBool>,
    active_phrases: Arc<Mutex<Vec<String>>>,
    tx: mpsc::Sender<WorkerEvent>,
) {
    std::thread::spawn(move || {
        let mut engine: Option<TranscribeCppEngine> = None;
        let mut mic_open = false;

        while !stop_flag.load(Ordering::Acquire) {
            if model_dirty.swap(false, Ordering::AcqRel) {
                let maybe_model = desired_model.lock().ok().and_then(|g| g.clone());
                match maybe_model {
                    Some(path) => match TranscribeCppEngine::load(&path) {
                        Ok(loaded) => {
                            let arch = loaded.model_arch();
                            engine = Some(loaded);
                            mic_open = false;
                            device_dirty.store(true, Ordering::Release);
                            let _ = tx.send(WorkerEvent::Status(format!(
                                "loaded model ({arch}): {}",
                                path.file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("model")
                            )));
                        }
                        Err(err) => {
                            engine = None;
                            mic_open = false;
                            let _ = tx.send(WorkerEvent::Error(err.to_string()));
                        }
                    },
                    None => {
                        engine = None;
                        mic_open = false;
                    }
                }
            }

            let Some(eng) = engine.as_mut() else {
                std::thread::sleep(Duration::from_millis(80));
                continue;
            };

            if !listening_flag.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }

            if device_dirty.swap(false, Ordering::AcqRel) || !mic_open {
                let sel = desired_device.lock().ok().and_then(|g| g.clone());
                match eng.with_microphone(sel.as_deref(), VadConfig::default()) {
                    Ok(()) => {
                        mic_open = true;
                        let name = eng.microphone_device_name().unwrap_or("default");
                        let _ = tx.send(WorkerEvent::Status(format!("mic ready: {name}")));
                    }
                    Err(err) => {
                        mic_open = false;
                        let _ = tx.send(WorkerEvent::Error(err.to_string()));
                        std::thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                }
            }

            let phrases = active_phrases
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();

            match eng.poll_recognize_command(&phrases, Duration::from_millis(200)) {
                Ok(Some(cands)) if !cands.is_empty() => {
                    let _ = tx.send(WorkerEvent::Candidates(cands));
                }
                Ok(_) => {}
                Err(err) => {
                    let _ = tx.send(WorkerEvent::Error(err.to_string()));
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
    });
}
