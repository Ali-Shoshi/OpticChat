mod api;
mod bounds;
mod buffer;
mod capture;
mod config;
mod desktop;
mod reply;
mod ui_check;
mod voice;
use anyhow::Result;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use std::{
    cell::Cell,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
slint::include_modules!();
type SettingsWindow = InputWindow;
type BubbleWindow = InputWindow;

fn color(s: &str) -> slint::Color {
    let (r, g, b) = config::parse_color(s).unwrap_or((20, 24, 32));
    slint::Color::from_rgb_u8(r, g, b)
}
macro_rules! theme {
    ($ui:expr,$c:expr) => {{
        let g = $ui.global::<Colors>();
        g.set_bg(color(&$c.background));
        g.set_fg(color(&$c.foreground));
        g.set_accent(color(&$c.accent));
    }};
}
fn filter_models(ui: &SettingsWindow, ids: &[String]) {
    let query = ui.get_model_filter().trim().to_lowercase();
    let filtered: Vec<SharedString> = ids
        .iter()
        .filter(|id| id.to_lowercase().contains(&query))
        .map(|s| s.as_str().into())
        .collect();
    let count = filtered.len();
    ui.set_models(ModelRc::new(VecModel::from(filtered)));
    ui.set_models_message(if ids.is_empty() {
        "No models returned for this API key.".into()
    } else {
        format!(
            "{count} of {} available models. Click one to select it.",
            ids.len()
        )
        .into()
    });
}

fn fill_settings(ui: &SettingsWindow, c: &config::Config) {
    ui.set_system_prompt(c.system_prompt.clone().into());
    ui.set_keep_on_top(c.always_on_top);
    ui.set_response_font_size(c.response_font_size as f32);
    ui.set_lmstudio_url(c.lmstudio_url.clone().into());
    ui.set_model_lmstudio(c.lmstudio_model.clone().into());
    ui.set_provider_index(if c.provider == "lmstudio" {
        3
    } else if c.provider == "hermes" {
        2
    } else if c.provider == "anthropic" {
        1
    } else {
        0
    });
    ui.set_model_openai(c.openai_model.clone().into());
    ui.set_model_anthropic(c.anthropic_model.clone().into());
    ui.set_shortcut_window(c.shortcuts[0].clone().into());
    ui.set_shortcut_response(c.shortcuts[1].clone().into());
    ui.set_shortcut_tl(c.shortcuts[2].clone().into());
    ui.set_shortcut_tr(c.shortcuts[3].clone().into());
    ui.set_shortcut_bl(c.shortcuts[4].clone().into());
    ui.set_shortcut_br(c.shortcuts[5].clone().into());
    ui.set_shortcut_input(c.shortcuts[6].clone().into());
    ui.set_shortcut_mic(c.shortcuts[7].clone().into());
    ui.set_model(c.model.clone().into());
    ui.set_bg_hex(c.background.clone().into());
    ui.set_fg_hex(c.foreground.clone().into());
    ui.set_accent_hex(c.accent.clone().into());
    ui.set_bubble_w(c.bubble_width.to_string().into());
    ui.set_bubble_h(c.bubble_height.to_string().into());
    ui.set_bubble_alpha(c.bubble_opacity * 100.);
    ui.set_interval(c.sample_seconds.to_string().into());
    ui.set_threshold(c.change_percent.to_string().into());
    ui.set_image_width(c.image_width.to_string().into());
    ui.set_quality(c.jpeg_quality.to_string().into());
}
fn read_settings(ui: &SettingsWindow) -> Result<config::Config> {
    let c = config::Config {
        system_prompt: ui.get_system_prompt().to_string(),
        always_on_top: ui.get_keep_on_top(),
        response_font_size: ui.get_response_font_size().round() as u32,
        model: ui.get_model().trim().into(),
        lmstudio_url: ui.get_lmstudio_url().trim().into(),
        lmstudio_model: if ui.get_provider_index() == 3 {
            ui.get_model()
        } else {
            ui.get_model_lmstudio()
        }
        .to_string(),
        provider: if ui.get_provider_index() == 3 {
            "lmstudio"
        } else if ui.get_provider_index() == 2 {
            "hermes"
        } else if ui.get_provider_index() == 1 {
            "anthropic"
        } else {
            "openai"
        }
        .into(),
        openai_model: if ui.get_provider_index() == 0 {
            ui.get_model()
        } else {
            ui.get_model_openai()
        }
        .to_string(),
        anthropic_model: if ui.get_provider_index() == 1 {
            ui.get_model()
        } else {
            ui.get_model_anthropic()
        }
        .to_string(),
        shortcuts: vec![
            ui.get_shortcut_window(),
            ui.get_shortcut_response(),
            ui.get_shortcut_tl(),
            ui.get_shortcut_tr(),
            ui.get_shortcut_bl(),
            ui.get_shortcut_br(),
            ui.get_shortcut_input(),
            ui.get_shortcut_mic(),
        ]
        .into_iter()
        .map(|s| s.trim().to_owned())
        .collect(),
        background: ui.get_bg_hex().trim().into(),
        foreground: ui.get_fg_hex().trim().into(),
        accent: ui.get_accent_hex().trim().into(),
        bubble_width: ui
            .get_bubble_w()
            .parse()
            .map_err(|_| anyhow::anyhow!("Enter a numeric bubble width"))?,
        bubble_height: ui
            .get_bubble_h()
            .parse()
            .map_err(|_| anyhow::anyhow!("Enter a numeric bubble height"))?,
        bubble_opacity: ui.get_bubble_alpha() / 100.,
        sample_seconds: ui
            .get_interval()
            .parse()
            .map_err(|_| anyhow::anyhow!("Enter a numeric interval"))?,
        change_percent: ui
            .get_threshold()
            .parse()
            .map_err(|_| anyhow::anyhow!("Enter a numeric change threshold"))?,
        image_width: ui
            .get_image_width()
            .parse()
            .map_err(|_| anyhow::anyhow!("Enter a numeric image width"))?,
        jpeg_quality: ui
            .get_quality()
            .parse()
            .map_err(|_| anyhow::anyhow!("Enter a numeric JPEG quality"))?,
    };
    c.validate()?;
    Ok(c)
}
fn apply_bubble(ui: &BubbleWindow, c: &config::Config) {
    ui.set_bubble_width(c.bubble_width as i32);
    ui.set_bubble_height(c.bubble_height as i32);
}

fn focus_input(ui: &InputWindow) {
    use slint::winit_030::WinitWindowAccessor;
    let delay = if ui.get_settings_open() { 460 } else { 20 };
    ui.set_settings_open(false);
    let _ = ui.show();
    ui.window()
        .with_winit_window(|native| native.focus_window());
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(delay), move || {
        if let Some(ui) = weak.upgrade() {
            ui.window()
                .with_winit_window(|native| native.focus_window());
            ui.invoke_focus_editor();
        }
    });
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let control = if args.iter().any(|a| a == "--toggle-mic") {
        Some("mic".to_owned())
    } else if args.iter().any(|a| a == "--focus-input") {
        Some("focus".to_owned())
    } else if args.iter().any(|a| a == "--toggle-window") {
        Some("window".to_owned())
    } else {
        args.iter().find_map(|a| {
            a.strip_prefix("--corner=")
                .or_else(|| a.strip_prefix("--move="))
                .map(|c| format!("corner:{c}"))
        })
    };
    if let Some(command) = &control {
        if desktop::send(command).is_ok() {
            return Ok(());
        }
    }
    if args
        .iter()
        .any(|s| s == "--toggle-bubble" || s == "--status")
    {
        let command = if args.iter().any(|s| s == "--status") {
            "status"
        } else {
            "toggle"
        };
        println!("{}", desktop::send(command)?);
        return Ok(());
    }
    if args.iter().any(|s| s == "--install-shortcut") {
        desktop::install_shortcut()?;
        println!("Optic Chat shortcuts registered");
        return Ok(());
    }
    if desktop::send("show").is_ok() {
        return Ok(());
    }
    // XWayland lets GNOME honor always-on-top. Capture still uses the Wayland portal.
    use slint::winit_030::winit::platform::x11::EventLoopBuilderExtX11;
    let mut event_loop = slint::winit_030::winit::event_loop::EventLoop::<
        slint::winit_030::SlintEvent,
    >::with_user_event();
    event_loop.with_x11();
    slint::BackendSelector::new()
        .with_winit_event_loop_builder(event_loop)
        .backend_name("winit".into())
        .renderer_name("femtovg".into())
        .select()?;

    let initial = config::Config::load();
    let load_error = initial.as_ref().err().map(|e| e.to_string());
    let cfg = Arc::new(Mutex::new(initial.unwrap_or_default()));
    let input = InputWindow::new()?;
    input.on_format_reply(|text| reply::formatted(&text));
    let voice = voice::Voice::default();
    {
        let controller = voice.clone();
        let weak = input.as_weak();
        input.on_toggle_mic(move || {
            let Some(ui) = weak.upgrade() else { return };
            if ui.get_recording() {
                ui.set_recording(false);
                ui.set_transcribing(true);
                ui.set_voice_status("Transcribing locally…".into());
                if let Err(error) = controller.stop() {
                    ui.set_transcribing(false);
                    ui.set_voice_status(format!("Microphone: {error}").into());
                }
                return;
            }
            if ui.get_transcribing() || ui.get_busy() { return; }
            ui.set_recording(true);
            ui.set_voice_status("Listening… Press the mic again to stop (maximum 2 minutes).".into());
            let weak = weak.clone();
            if let Err(error) = controller.start(move |event| {
                let weak = weak.clone();
                let _ = slint::invoke_from_event_loop(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    if event["phase"] == "transcribing" {
                        ui.set_recording(false);
                        ui.set_transcribing(true);
                        ui.set_voice_status("Transcribing locally…".into());
                    } else if let Some(text) = event["text"].as_str() {
                        ui.set_recording(false);
                        ui.set_transcribing(false);
                        if text.trim().is_empty() {
                            ui.set_voice_status("No speech detected. Try recording again.".into());
                        } else {
                            let previous = ui.get_question();
                            ui.set_question(if previous.trim().is_empty() { text.into() } else { format!("{} {}", previous.trim_end(), text).into() });
                            ui.set_voice_status("Dictation added — review it, then press Send.".into());
                            focus_input(&ui);
                        }
                    } else if let Some(error) = event["error"].as_str() {
                        ui.set_recording(false);
                        ui.set_transcribing(false);
                        ui.set_voice_status(format!("Microphone: {error}").into());
                    }
                });
            }) {
                ui.set_recording(false);
                ui.set_voice_status(format!("Microphone: {error}").into());
            }
        });
    }
    {
        let w = input.as_weak();
        input.on_drag(move || {
            bounds::cancel_animation();
            if let Some(w) = w.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                w.window().with_winit_window(|native| {
                    let _ = native.drag_window();
                });
            }
        });
    }

    let screen_guard = slint::Timer::default();
    {
        let w = input.as_weak();
        let work = bounds::WorkArea::new();
        let cached_area = Cell::new(None);
        let area_tick = Cell::new(0u32);
        let applied_level = Cell::new(None);
        screen_guard.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(150),
            move || {
                use slint::winit_030::WinitWindowAccessor;
                if let Some(ui) = w.upgrade() {
                    ui.window().with_winit_window(|native| {
                        // Initial X11 window hints can be lost while GNOME maps
                        // the window. Apply the level once the window is shown,
                        // and again after restoring it or changing the checkbox.
                        if native.is_visible() == Some(true) {
                            let keep_on_top = ui.get_keep_on_top();
                            if applied_level.get() != Some(keep_on_top) {
                                use slint::winit_030::winit::window::WindowLevel;
                                native.set_window_level(if keep_on_top {
                                    WindowLevel::AlwaysOnTop
                                } else {
                                    WindowLevel::Normal
                                });
                                applied_level.set(Some(keep_on_top));
                            }
                        } else {
                            applied_level.set(None);
                        }
                        let Some(monitor) = native
                            .current_monitor()
                            .or_else(|| native.primary_monitor())
                        else {
                            return;
                        };
                        let p = monitor.position();
                        let size = monitor.size();
                        let mut r = bounds::Rect {
                            x: p.x,
                            y: p.y,
                            w: size.width as i32,
                            h: size.height as i32,
                        };
                        if area_tick.get() % 7 == 0
                            || cached_area.get().is_none_or(|(monitor, _)| monitor != r)
                        {
                            cached_area.set(Some((
                                r,
                                work.as_ref().map(|work| work.for_monitor(r)).unwrap_or(r),
                            )));
                        }
                        area_tick.set(area_tick.get().wrapping_add(1));
                        if let Some((_, safe)) = cached_area.get() {
                            r = safe;
                        }
                        let scale = native.scale_factor();
                        ui.set_screen_width((r.w as f64 / scale) as f32);
                        ui.set_screen_height((r.h as f64 / scale) as f32);
                        if let Ok(p) = native.outer_position() {
                            let size = native.outer_size();
                            let (x, y) = bounds::position(
                                r,
                                p.x,
                                p.y,
                                size.width as i32,
                                size.height as i32,
                            );
                            if (x, y) != (p.x, p.y) {
                                native.set_outer_position(
                                    slint::winit_030::winit::dpi::PhysicalPosition::new(x, y),
                                );
                            }
                        }
                    });
                }
            },
        );
    }
    {
        let w = input.as_weak();
        input.on_preview_colors(move || {
            if let Some(ui) = w.upgrade() {
                if let Some(c) = config::parse_color(&ui.get_bg_hex()) {
                    ui.global::<Colors>()
                        .set_bg(slint::Color::from_rgb_u8(c.0, c.1, c.2));
                }
                if let Some(c) = config::parse_color(&ui.get_fg_hex()) {
                    ui.global::<Colors>()
                        .set_fg(slint::Color::from_rgb_u8(c.0, c.1, c.2));
                }
                if let Some(c) = config::parse_color(&ui.get_accent_hex()) {
                    ui.global::<Colors>()
                        .set_accent(slint::Color::from_rgb_u8(c.0, c.1, c.2));
                }
            }
        });
    }
    let settings = input.clone_strong();
    let bubble = input.clone_strong();
    {
        let c = cfg.lock().unwrap();
        theme!(&input, &c);
        theme!(&settings, &c);
        theme!(&bubble, &c);
        fill_settings(&settings, &c);
        apply_bubble(&bubble, &c);
    }
    if let Some(e) = load_error {
        settings.set_message(format!("Settings could not be loaded: {e}").into());
    }
    let rt = Arc::new(tokio::runtime::Runtime::new()?);
    let buffer = Arc::new(Mutex::new(buffer::FrameBuffer::default()));
    let history = Arc::new(Mutex::new(api::Conversation::default()));
    let capture_stop = Arc::new(AtomicBool::new(true));
    let capture_refresh = Arc::new(AtomicBool::new(false));
    let alive = Arc::new(AtomicBool::new(true));
    let request_cancel = Arc::new(Mutex::new(Arc::new(AtomicBool::new(false))));
    let bubble_visible = Arc::new(AtomicBool::new(false));
    let toggle: Rc<dyn Fn()> = {
        let b = bubble.as_weak();
        let visible = bubble_visible.clone();
        Rc::new(move || {
            if let Some(b) = b.upgrade() {
                if b.get_reply_visible() {
                    b.set_reply_visible(false);
                    visible.store(false, Ordering::Relaxed);
                } else {
                    b.set_reply_visible(true);
                    visible.store(true, Ordering::Relaxed);
                }
            }
        })
    };
    {
        let f = toggle.clone();
        input.on_toggle_bubble(move || f());
    }
    {
        let b = bubble.as_weak();
        let v = bubble_visible.clone();
        bubble.on_dismiss(move || {
            if let Some(b) = b.upgrade() {
                b.set_reply_visible(false);
                v.store(false, Ordering::Relaxed);
            }
        });
    }
    {
        let v = bubble_visible.clone();
        bubble.window().on_close_requested(move || {
            v.store(false, Ordering::Relaxed);
            slint::CloseRequestResponse::HideWindow
        });
    }
    settings
        .window()
        .on_close_requested(|| slint::CloseRequestResponse::HideWindow);
    input.window().on_close_requested(|| {
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    {
        let w = settings.as_weak();
        input.on_settings(move || {
            if let Some(w) = w.upgrade() {
                w.set_settings_open(!w.get_settings_open());
                if w.get_settings_open()
                    && !w.get_models_loading()
                    && w.get_models().row_count() == 0
                    && config::provider_key(
                        if w.get_provider_index() == 3 {
                            "lmstudio"
                        } else if w.get_provider_index() == 2 {
                            "hermes"
                        } else if w.get_provider_index() == 1 {
                            "anthropic"
                        } else {
                            "openai"
                        },
                        "",
                    )
                    .is_ok()
                {
                    w.invoke_fetch_models();
                }
            }
        });
    }
    {
        let stop = request_cancel.clone();
        input.on_stop(move || stop.lock().unwrap().store(true, Ordering::Relaxed));
    }
    {
        let buffer = buffer.clone();
        let stop = capture_stop.clone();
        let w = settings.as_weak();
        settings.on_clear_frames(move || {
            capture::stop_and_clear(&stop, &buffer);
            if let Some(w) = w.upgrade() {
                w.set_message("Screenshots cleared and sharing stopped. Click + to share again.".into());
                w.set_status("Screenshots cleared • 0/10 frames • sharing stopped".into());
            }
        });
    }
    {
        let history = history.clone();
        let w = settings.as_weak();
        settings.on_new_chat(move || {
            history.lock().unwrap().turns.clear();
            if let Some(w) = w.upgrade() {
                w.set_answer("Your reply will appear here.".into());
                w.set_current_question("".into());
                w.set_past_turns(ModelRc::new(VecModel::<ChatTurn>::default()));
                w.set_message("Conversation cleared. The next question starts fresh.".into());
            }
        });
    }
    {
        let w = settings.as_weak();
        settings.on_install_shortcut(move || {
            let text = match w
                .upgrade()
                .ok_or_else(|| anyhow::anyhow!("Window closed"))
                .and_then(|ui| {
                    desktop::install_shortcuts(
                        &vec![
                            ui.get_shortcut_window(),
                            ui.get_shortcut_response(),
                            ui.get_shortcut_tl(),
                            ui.get_shortcut_tr(),
                            ui.get_shortcut_bl(),
                            ui.get_shortcut_br(),
                            ui.get_shortcut_input(),
                            ui.get_shortcut_mic(),
                        ]
                        .into_iter()
                        .map(String::from)
                        .collect::<Vec<_>>(),
                    )
                }) {
                Ok(()) => "Shortcuts applied. Save settings to keep these bindings.".into(),
                Err(e) => e.to_string(),
            };
            if let Some(w) = w.upgrade() {
                w.set_message(text.into());
            }
        });
    }
    input.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
    let model_debounce = Rc::new(slint::Timer::default());
    let model_generation = Arc::new(AtomicU64::new(0));
    let available_models = Arc::new(Mutex::new(Vec::<String>::new()));
    {
        let w = settings.as_weak();
        let timer = model_debounce.clone();
        let generation = model_generation.clone();
        let ids = available_models.clone();
        settings.on_key_edited(move || {
            timer.stop();
            generation.fetch_add(1, Ordering::Relaxed);
            if let Some(ui) = w.upgrade() {
                ui.set_models_loading(false);
                ids.lock().unwrap().clear();
                ui.set_models(ModelRc::new(VecModel::<SharedString>::default()));
                ui.set_models_message("Checking your key when you finish typing…".into());
                if ui.get_key().trim().is_empty() {
                    ui.set_models_message("Paste a key, or refresh to use your saved key.".into());
                    return;
                }
                let w = w.clone();
                timer.start(
                    slint::TimerMode::SingleShot,
                    std::time::Duration::from_millis(900),
                    move || {
                        if let Some(ui) = w.upgrade() {
                            ui.invoke_fetch_models();
                        }
                    },
                );
            }
        });
    }
    {
        let w = settings.as_weak();
        let ids = available_models.clone();
        settings.on_filter_models(move || {
            if let Some(ui) = w.upgrade() {
                filter_models(&ui, &ids.lock().unwrap());
            }
        });
    }
    {
        let w = settings.as_weak();
        let i = input.as_weak();
        let b = bubble.as_weak();
        let cfg = cfg.clone();
        settings.on_save(move || {
            if let Some(w) = w.upgrade() {
                let result: Result<()> = (|| {
                    let c = read_settings(&w)?;
                    if c.provider != "hermes" && !w.get_key().trim().is_empty() {
                        config::save_key(&c.provider, w.get_key().trim())?;
                        w.set_key("".into());
                    }
                    if cfg.lock().unwrap().shortcuts != c.shortcuts {
                        desktop::install_shortcuts(&c.shortcuts)?;
                    }
                    c.save()?;
                    theme!(&w, &c);
                    if let Some(i) = i.upgrade() {
                        theme!(&i, &c);
                    }
                    if let Some(b) = b.upgrade() {
                        theme!(&b, &c);
                        apply_bubble(&b, &c);
                    }
                    *cfg.lock().unwrap() = c;
                    Ok(())
                })();
                let saved = result.is_ok();
                w.set_message(match result {
                    Ok(()) => "Saved. Re-share to apply new capture resolution or quality.".into(),
                    Err(e) => format!("Could not save: {e}").into(),
                });
                if saved && w.get_models().row_count() == 0 {
                    w.invoke_fetch_models();
                }
            }
        });
    }
    {
        let w = settings.as_weak();
        let generation = model_generation.clone();
        let timer = model_debounce.clone();
        let ids = available_models.clone();
        let previous = Cell::new(settings.get_provider_index());
        let drafts =
            std::cell::RefCell::new([String::new(), String::new(), String::new(), String::new()]);
        settings.on_provider_changed(move || {
            if let Some(ui) = w.upgrade() {
                let old = previous.replace(ui.get_provider_index());
                drafts.borrow_mut()[old as usize] = ui.get_key().to_string();
                if old == 0 {
                    ui.set_model_openai(ui.get_model());
                } else if old == 1 {
                    ui.set_model_anthropic(ui.get_model());
                } else if old == 3 {
                    ui.set_model_lmstudio(ui.get_model());
                }
                timer.stop();
                generation.fetch_add(1, Ordering::Relaxed);
                ids.lock().unwrap().clear();
                ui.set_models_loading(false);
                ui.set_models(ModelRc::new(VecModel::<SharedString>::default()));
                ui.set_model_filter("".into());
                ui.set_key(
                    drafts.borrow()[ui.get_provider_index() as usize]
                        .clone()
                        .into(),
                );
                ui.set_model(if ui.get_provider_index() == 3 {
                    ui.get_model_lmstudio()
                } else if ui.get_provider_index() == 2 {
                    "gpt-6.1-sol".into()
                } else if ui.get_provider_index() == 0 {
                    ui.get_model_openai()
                } else {
                    ui.get_model_anthropic()
                });
                ui.set_models_message(
                    "Refresh models to connect. Add a token only if this provider requires one.".into(),
                );
                let provider = if ui.get_provider_index() == 3 {
                    "lmstudio"
                } else if ui.get_provider_index() == 2 {
                    "hermes"
                } else if ui.get_provider_index() == 1 {
                    "anthropic"
                } else {
                    "openai"
                };
                if config::provider_key(provider, ui.get_key().trim()).is_ok() {
                    ui.invoke_fetch_models();
                }
            }
        });
    }
    {
        let w = settings.as_weak();
        let rt = rt.clone();
        let generation = model_generation.clone();
        let all_ids = available_models.clone();
        let timer = model_debounce.clone();
        settings.on_fetch_models(move||{
            timer.stop();let Some(ui)=w.upgrade() else{return};
            if ui.get_models_loading(){return;}
            let provider=if ui.get_provider_index()==3 {"lmstudio"}else if ui.get_provider_index()==2 {"hermes"}else if ui.get_provider_index()==1 {"anthropic"}else{"openai"};
            let key=match config::provider_key(provider,ui.get_key().trim()){
                Ok(key)=>key,Err(e)=>{ui.set_models_message(e.to_string().into());return;}
            };
            let request_id=generation.fetch_add(1,Ordering::Relaxed)+1;
            ui.set_models_loading(true);ui.set_models_message("Loading available models…".into());
            let local_url=ui.get_lmstudio_url().to_string();
            let w=w.clone();let generation=generation.clone();let all_ids=all_ids.clone();
            rt.spawn(async move{
                let result=api::provider_models(provider,&key,&local_url).await;
                let _=slint::invoke_from_event_loop(move||{
                    if generation.load(Ordering::Relaxed)!=request_id{return;}
                    if let Some(ui)=w.upgrade(){
                        ui.set_models_loading(false);
                        match result{
                            Ok(ids)=>{
                                filter_models(&ui,&ids);
                                *all_ids.lock().unwrap()=ids;
                                let reveal=ui.as_weak();
                                slint::Timer::single_shot(std::time::Duration::from_millis(100),move||{
                                    if let Some(ui)=reveal.upgrade(){
                                        if ui.get_settings_open() && ui.get_tab()==0 {ui.invoke_reveal_models();}
                                    }
                                });
                            },
                            Err(e)=>{all_ids.lock().unwrap().clear();ui.set_models(ModelRc::new(VecModel::<SharedString>::default()));ui.set_models_message(format!("Could not load models: {e}. Check your key and try Refresh models.").into());}
                        }
                    }
                });
            });
        });
    }
    {
        let w = input.as_weak();
        let rt = rt.clone();
        let cfg = cfg.clone();
        let buffer = buffer.clone();
        let stop = capture_stop.clone();
        let refresh = capture_refresh.clone();
        input.on_toggle_share(move || {
            let Some(ui) = w.upgrade() else { return };
            if ui.get_sharing() {
                stop.store(true, Ordering::Relaxed);
                ui.set_status("Pausing screen sharing…".into());
                return;
            }
            ui.set_sharing(true);
            buffer.lock().unwrap().clear();
            refresh.store(false, Ordering::Relaxed);
            stop.store(false, Ordering::Relaxed);
            let w = w.clone();
            let cfg = cfg.clone();
            let buffer = buffer.clone();
            let stop = stop.clone();
            let refresh = refresh.clone();
            rt.spawn(async move {
                let ws = w.clone();
                let status_stop = stop.clone();
                let result = capture::run(cfg, buffer.clone(), stop.clone(), refresh, move |s| {
                    let w = ws.clone();
                    let stop = status_stop.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = w.upgrade().filter(|_| !stop.load(Ordering::Relaxed)) {
                            w.set_status(s.into());
                        }
                    });
                })
                .await;
                capture::stop_and_clear(&stop, &buffer);
                let text = match result {
                    Ok(()) => "Sharing stopped • 0/10 frames".into(),
                    Err(e) => format!("Capture: {e} • buffer cleared"),
                };
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(ui) = w.upgrade() {
                        ui.set_sharing(false);
                        ui.set_status(text.into());
                    }
                });
            });
        });
    }
    {
        let w = input.as_weak();
        let b = bubble.as_weak();
        let sw = settings.as_weak();
        let rt = rt.clone();
        let cfg = cfg.clone();
        let buffer = buffer.clone();
        let history = history.clone();
        let cancels = request_cancel.clone();
        let visible = bubble_visible.clone();
        let capture_stop = capture_stop.clone();
        let refresh = capture_refresh.clone();
        input.on_send(move || {
            let Some(ui) = w.upgrade() else { return };
            if ui.get_busy() || ui.get_recording() || ui.get_transcribing() {
                return;
            }
            let question = ui.get_question().trim().to_string();
            if question.is_empty() {
                return;
            }
            let c = cfg.lock().unwrap().clone();
            let key = match config::provider_key(&c.provider, "") {
                Ok(k) => k,
                Err(e) => {
                    if let Some(s) = sw.upgrade() {
                        s.set_message(e.to_string().into());
                        s.set_settings_open(true);
                    }
                    return;
                }
            };
            let h = history.lock().unwrap().clone();
            let cancel = Arc::new(AtomicBool::new(false));
            *cancels.lock().unwrap() = cancel.clone();
            ui.set_busy(true);
            ui.set_question("".into());
            if let Some(b) = b.upgrade() {
                b.set_past_turns(ModelRc::new(VecModel::from(h.turns.iter().map(|(q,a)| ChatTurn { question:q.as_str().into(), answer:a.as_str().into() }).collect::<Vec<_>>())));
                b.set_current_question(question.clone().into());
                b.set_busy(true);
                b.set_answer(if capture_stop.load(Ordering::Relaxed) { "Thinking…" } else { "Getting the latest screen…" }.into());
                b.set_reply_visible(true);
                let weak = b.as_weak();
                slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
                    if let Some(ui) = weak.upgrade() { ui.invoke_reveal_response(); }
                });
                visible.store(true, Ordering::Relaxed);
            }
            let w = w.clone();
            let b = b.clone();
            let history = history.clone();
            let buffer = buffer.clone();
            let capture_stop = capture_stop.clone();
            let refresh = refresh.clone();
            rt.spawn(async move {
                let bw = b.clone();
                let mut first_chunk = true;
                let result: Result<String> = async {
                    let frames = capture::fresh_frames(&buffer, &capture_stop, &refresh, &cancel).await?;
                    let mut body = api::provider_body_with_prompt(&c.provider, &c.model, &question, &frames, &h, &c.system_prompt);
                    if c.provider == "lmstudio" {
                        body["optic_lmstudio_url"] = serde_json::json!(c.lmstudio_url);
                    }
                    api::ask(key, body, cancel, move |answer| {
                        let b = bw.clone();
                        let reveal = first_chunk;
                        first_chunk = false;
                        let _ = slint::invoke_from_event_loop(move || {
                            if let Some(b) = b.upgrade() {
                                b.set_answer(answer.into());
                                if reveal {
                                    let weak = b.as_weak();
                                    slint::Timer::single_shot(std::time::Duration::from_millis(30), move || {
                                        if let Some(ui) = weak.upgrade() { ui.invoke_reveal_response(); }
                                    });
                                }
                            }
                        });
                    }).await
                }.await;
                if let Ok(answer) = &result {
                    history
                        .lock()
                        .unwrap()
                        .record(question.clone(), answer.clone());
                }
                let _ = slint::invoke_from_event_loop(move || {
                    if let Some(w) = w.upgrade() {
                        w.set_busy(false);
                        if result.is_err() {
                            w.set_question(question.into());
                        }
                    }
                    if let Some(b) = b.upgrade() {
                        b.set_busy(false);
                        if let Err(e) = result {
                            b.set_answer(
                                format!("{e}\n\nCheck Settings → Connection, or try again.").into(),
                            );
                        }
                    }
                });
            });
        });
    }

    // IPC uses a private user-only socket, also used by the GNOME shortcut.
    {
        let w = input.as_weak();
        let b = bubble.as_weak();
        let buffer = buffer.clone();
        let visible = bubble_visible.clone();
        let capture_stop = capture_stop.clone();
        desktop::listen(alive.clone(), move |command| {
            if command == "status" {
                let frames = buffer.lock().unwrap();
                let (count, bytes) = frames.stats();
                return serde_json::json!({"running":true,"buffered_frames":count,"buffer_bytes":bytes,"latest_frame_age_seconds":frames.latest_age_seconds(),"capture_active":!capture_stop.load(Ordering::Relaxed),"bubble_visible":visible.load(Ordering::Relaxed)}).to_string();
            }
            let w = w.clone();
            let b = b.clone();
            let visible = visible.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if command == "toggle" {
                    if let Some(b) = b.upgrade() {
                        if b.get_reply_visible() {
                            b.set_reply_visible(false);
                            visible.store(false, Ordering::Relaxed);
                        } else {
                            b.set_reply_visible(true);
                            visible.store(true, Ordering::Relaxed);
                        }
                    }
                } else if command == "mic" {
                    if let Some(w) = w.upgrade() {
                        focus_input(&w);
                        w.invoke_toggle_mic();
                    }
                } else if command == "focus" {
                    if let Some(w) = w.upgrade() {
                        focus_input(&w);
                    }
                } else if command == "window" {
                    if let Some(w) = w.upgrade() {
                        if w.window().is_visible() {
                            let _ = w.hide();
                        } else {
                            let _ = w.show();
                        }
                    }
                } else if let Some(corner) = command.strip_prefix("corner:") {
                    if let Some(w) = w.upgrade() {
                        let _ = w.show();
                        bounds::move_corner(&w, corner);
                    }
                } else if command == "show" {
                    if let Some(w) = w.upgrade() {
                        let _ = w.show();
                    }
                }
            });
            "ok".into()
        })?;
    }
    if let Some(command) = control {
        if command == "mic" {
            let w = input.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
                if let Some(w) = w.upgrade() { focus_input(&w); w.invoke_toggle_mic(); }
            });
        }
        if command == "focus" {
            let w = input.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
                if let Some(w) = w.upgrade() {
                    focus_input(&w);
                }
            });
        }
        if command.starts_with("corner:") {
            let w = input.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
                if let Some(w) = w.upgrade() {
                    bounds::move_corner(&w, command.trim_start_matches("corner:"));
                }
            });
        }
    }
    if let Ok(directory) = std::env::var("OPTIC_CHAT_VOICE_CHECK_DIR") {
        ui_check::voice_input(&input, directory)?;
    } else if let Ok(directory) = std::env::var("OPTIC_CHAT_PROMPT_CHECK_DIR") {
        ui_check::prompt_settings(&input, directory)?;
    } else if let Ok(directory) = std::env::var("OPTIC_CHAT_REPLY_CHECK_DIR") {
        ui_check::reply_format(&input, directory)?;
    } else if std::env::var_os("OPTIC_CHAT_FOCUS_CHECK").is_some() {
        ui_check::focus_and_bounds(&input)?;
    } else if let Ok(directory) = std::env::var("OPTIC_CHAT_SETTINGS_CHECK_DIR") {
        ui_check::settings_update(&input, directory)?;
    } else if let Ok(directory) = std::env::var("OPTIC_CHAT_SCROLL_CHECK_DIR") {
        ui_check::run(&input, directory)?;
    } else if let Ok(directory) = std::env::var("OPTIC_CHAT_PREVIEW_DIR") {
        std::fs::create_dir_all(&directory)?;
        settings.set_settings_open(true);
        bubble.set_answer("1. Open the app’s Settings.\n\n2. Choose the option you want to change.\n\n3. Save, then ask me what to do next.\n\nThis is a UI preview, not an AI response.".into());
        bubble.set_reply_visible(true);
        let s = settings.as_weak();
        let i = input.as_weak();
        let b = bubble.as_weak();
        let timer = slint::Timer::default();
        let step = Rc::new(Cell::new(0));
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(1000),
            move || {
                let n = step.get();
                step.set(n + 1);
                let save = |window: &slint::Window, name: &str| match window.take_snapshot() {
                    Ok(frame) => {
                        let path = std::path::Path::new(&directory).join(name);
                        if let Err(e) = image::save_buffer(
                            path,
                            frame.as_bytes(),
                            frame.width(),
                            frame.height(),
                            image::ColorType::Rgba8,
                        ) {
                            eprintln!("Preview: {e}");
                        }
                    }
                    Err(e) => eprintln!("Preview: {e}"),
                };
                if let Some(s) = s.upgrade() {
                    if n < 3 {
                        save(s.window(), &format!("settings-{n}.png"));
                        s.set_tab((n + 1).min(2));
                        if n == 2 {
                            s.set_settings_open(false);
                        }
                    }
                }
                if n == 3 {
                    if let Some(i) = i.upgrade() {
                        save(i.window(), "input.png");
                    }
                    if let Some(b) = b.upgrade() {
                        save(b.window(), "bubble.png");
                    }
                    if let Some(i)=i.upgrade(){i.set_bubble_alpha(0.0);i.set_question("First line\nSecond line\nThird line".into());}
                }
                if n>=4 {if let Some(i)=i.upgrade(){
                    eprintln!("Preview step {n}: input height {}, window {:?}",i.get_input_height(),i.window().size());
                    save(i.window(),&format!("input-{n}.png"));
                    match n {
                        4=>i.set_question("First line\nSecond line\nThird line\nFourth line\nFifth line\nSixth line".into()),
                        5=>i.set_question("A long question should wrap naturally while I keep typing, so the input expands to show a second line and then a third line. After that it should scroll inside this same input instead of making the overlay taller and taller. ".repeat(4).into()),
                        6=>{i.set_question("".into());i.set_bubble_alpha(100.0);},
                        7=>{i.window().set_position(slint::PhysicalPosition::new(-5000,5000));},
                        8=>{
                            eprintln!("Clamped window position: {:?}",i.window().position());
                            use slint::platform::{WindowEvent,PointerEventButton};
                            let position=slint::LogicalPosition::new(80.0,28.0);
                            i.window().dispatch_event(WindowEvent::PointerPressed{position,button:PointerEventButton::Left});
                            i.window().dispatch_event(WindowEvent::PointerReleased{position,button:PointerEventButton::Left});
                            i.window().dispatch_event(WindowEvent::KeyPressed{text:"Typing a long question should wrap naturally. ".repeat(12).into()});
                            i.window().dispatch_event(WindowEvent::KeyPressed{text:"THE CURSOR SHOULD BE HERE".into()});
                        },
                        9=>{assert!(i.get_question().ends_with("THE CURSOR SHOULD BE HERE"));assert!(i.get_input_height()<=63.0);save(i.window(),"input-caret.png");i.set_settings_open(true);i.set_tab(1);},
                        10=>{i.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled{position:slint::LogicalPosition::new(280.0,300.0),delta_x:0.0,delta_y:-420.0});},
                        11=>{save(i.window(),"appearance-controls.png");let _=slint::quit_event_loop();},
                        _=>{}
                    }
                }}
            },
        );
        input.show()?;
        slint::run_event_loop()?;
        drop(timer);
    } else {
        input.show()?;
        slint::run_event_loop_until_quit()?;
    }
    let result: Result<(), slint::PlatformError> = Ok(());
    voice.shutdown();
    alive.store(false, Ordering::Relaxed);
    capture_stop.store(true, Ordering::Relaxed);
    request_cancel
        .lock()
        .unwrap()
        .store(true, Ordering::Relaxed);
    buffer.lock().unwrap().clear();
    drop(rt);
    result?;
    Ok(())
}
