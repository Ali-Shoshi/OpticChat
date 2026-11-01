//! Opt-in renderer regression checks; uses fixture text/models, never an API key.
use crate::InputWindow;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::Cell, path::Path, rc::Rc};
pub fn voice_input(ui: &InputWindow, directory: String) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    ui.set_question("Keep this draft.".into());
    let w = ui.as_weak();
    let step = Cell::new(0);
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
        let ui = w.upgrade().unwrap();
        let n = step.get(); step.set(n+1);
        match n {
            0 => ui.invoke_toggle_mic(),
            1 => {
                assert!(ui.get_recording());
                let frame = ui.window().take_snapshot().unwrap();
                image::save_buffer(Path::new(&directory).join("microphone-recording.png"), frame.as_bytes(), frame.width(), frame.height(), image::ColorType::Rgba8).unwrap();
                ui.invoke_toggle_mic();
                assert!(!ui.get_recording());
                assert!(ui.get_transcribing());
            }
            _ => {
                assert!(n < 30, "Transcription should finish");
                if !ui.get_transcribing() {
                    assert!(!ui.get_voice_status().starts_with("Microphone:"), "{}",ui.get_voice_status());
                    assert!(ui.get_question().starts_with("Keep this draft."));
                    eprintln!("Mic UI recording/stop/local transcription/draft preservation passed.");
                    // Exit during a new recording to exercise cleanup on quit.
                    ui.invoke_toggle_mic();
                    slint::Timer::single_shot(std::time::Duration::from_millis(400), || {slint::quit_event_loop().unwrap();});
                }
            }
        }
    });
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    drop(timer);
    Ok(())
}
pub fn prompt_settings(ui: &InputWindow, directory: String) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    ui.set_settings_open(true);
    ui.set_tab(2);
    let expected = ui.get_system_prompt();
    assert!(!expected.is_empty());
    let w = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(900), move || {
        let ui = w.upgrade().unwrap();
        let frame = ui.window().take_snapshot().unwrap();
        image::save_buffer(Path::new(&directory).join("prompt-settings.png"), frame.as_bytes(), frame.width(), frame.height(), image::ColorType::Rgba8).unwrap();
        use slint::platform::{WindowEvent, PointerEventButton};
        let position = slint::LogicalPosition::new(180.0,220.0);
        ui.window().dispatch_event(WindowEvent::PointerPressed {position,button:PointerEventButton::Left});
        ui.window().dispatch_event(WindowEvent::PointerReleased {position,button:PointerEventButton::Left});
        ui.window().dispatch_event(WindowEvent::KeyPressed {text:"PROMPT_EDIT_CHECK".into()});
        assert!(ui.get_system_prompt().contains("PROMPT_EDIT_CHECK"), "Prompt editor must accept typing");
        ui.set_system_prompt(expected);
        ui.window().dispatch_event(WindowEvent::PointerScrolled {position:slint::LogicalPosition::new(440.0,350.0),delta_x:0.0,delta_y:-100000.0});
        let w = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
            let ui = w.upgrade().unwrap();
            assert!(ui.get_last_control_gap() <= 1.0, "Clear buffer button must be fully reachable");
            let frame = ui.window().take_snapshot().unwrap();
            image::save_buffer(Path::new(&directory).join("prompt-settings-bottom.png"), frame.as_bytes(), frame.width(), frame.height(), image::ColorType::Rgba8).unwrap();
            slint::quit_event_loop().unwrap();
        });
    });
    ui.show()?;
    slint::run_event_loop()?;
    Ok(())
}
pub fn reply_format(ui: &InputWindow, directory: String) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    ui.set_settings_open(false);
    ui.set_reply_visible(true);
    ui.set_bubble_alpha(85.0);
    let mut history = crate::api::Conversation::default();
    history.record("How do I connect LM Studio?".into(), "## Connect locally\n\n1. Start the **local server**.\n2. Choose a model in **Connection**.\n3. Click **Save**.".into());
    ui.set_past_turns(ModelRc::new(VecModel::from(history.turns.iter().map(|(q,a)| crate::ChatTurn {question:q.as_str().into(),answer:a.as_str().into()}).collect::<Vec<_>>())));
    ui.set_current_question("How can I check it worked?".into());
    ui.set_answer("Look for **LM Studio** in Connection.\n\nAsk a short question, such as *Hello*.\n\n```text\nLocal server: http://127.0.0.1:1234\n```\n\nEnd of conversation.".into());
    let w = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(800), move || {
        let ui = w.upgrade().unwrap();
        let save = |name: &str| {
            let frame = ui.window().take_snapshot().unwrap();
            image::save_buffer(Path::new(&directory).join(name), frame.as_bytes(), frame.width(), frame.height(), image::ColorType::Rgba8).unwrap();
        };
        save("conversation-top.png");
        ui.invoke_reveal_response();
        let w = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
            let ui = w.upgrade().unwrap();
            assert!(ui.get_response_start_gap().abs() <= 1.0, "Response should start at the viewport top");
            let frame = ui.window().take_snapshot().unwrap();
            image::save_buffer(Path::new(&directory).join("response-start.png"), frame.as_bytes(), frame.width(), frame.height(), image::ColorType::Rgba8).unwrap();
            ui.set_response_font_size(28.0);
            let w = ui.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_millis(200), move || {
                let ui = w.upgrade().unwrap();
                ui.invoke_reveal_response();
                let w = ui.as_weak();
                slint::Timer::single_shot(std::time::Duration::from_millis(200), move || {
                    let ui = w.upgrade().unwrap();
                    assert!(ui.get_response_start_gap().abs() <= 1.0);
                    let frame = ui.window().take_snapshot().unwrap();
                    image::save_buffer(Path::new(&directory).join("response-large.png"), frame.as_bytes(), frame.width(), frame.height(), image::ColorType::Rgba8).unwrap();
                    slint::quit_event_loop().unwrap();
                });
            });
        });
    });
    ui.show()?;
    slint::run_event_loop()?;
    Ok(())
}
pub fn run(ui: &InputWindow, directory: String) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    ui.set_settings_open(true);
    ui.set_tab(1);
    let timer = slint::Timer::default();
    let w = ui.as_weak();
    let step = Rc::new(Cell::new(0));
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(1400),
        move || {
            let Some(ui) = w.upgrade() else { return };
            let n = step.get();
            step.set(n + 1);
            let save = |name: &str| {
                let frame = ui.window().take_snapshot().unwrap();
                image::save_buffer(
                    Path::new(&directory).join(name),
                    frame.as_bytes(),
                    frame.width(),
                    frame.height(),
                    image::ColorType::Rgba8,
                )
                .unwrap();
            };
            let scroll = |x, y| {
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                        position: slint::LogicalPosition::new(x, y),
                        delta_x: 0.0,
                        delta_y: -100000.0,
                    })
            };
            let click=|x,y|{
                use slint::platform::{WindowEvent,PointerEventButton};
                let position=slint::LogicalPosition::new(x,y);
                ui.window().dispatch_event(WindowEvent::PointerPressed{position,button:PointerEventButton::Left});
                ui.window().dispatch_event(WindowEvent::PointerReleased{position,button:PointerEventButton::Left});
            };
            match n {
                0 => {
                    save("appearance-top.png");
                    scroll(280.0, 300.0);
                }
                1 => {
                    eprintln!("Settings bottom gap: {}", ui.get_settings_scroll_gap());
                    assert!(ui.get_settings_scroll_gap() <= 1.0);
                    eprintln!("Appearance last control gap {}", ui.get_last_control_gap());
                    assert!(ui.get_last_control_gap() <= 0.0);
                    save("appearance-bottom.png");
                    ui.set_settings_open(false);
                    ui.set_reply_visible(true);
                    ui.set_bubble_alpha(35.0);
                    ui.set_answer(
                        ((1..=60)
                            .map(|n| format!("Line {n}: This is a long reply scroll check.\\n"))
                            .collect::<String>()
                            + "END OF REPLY — visible at the bottom.")
                            .replace("\\n", "\n")
                            .into(),
                    );
                }
                2 => {
                    save("reply-top.png");
                    scroll(280.0, 220.0);
                }
                3 => {
                    eprintln!("Reply bottom gap: {}", ui.get_reply_scroll_gap());
                    assert!(ui.get_reply_scroll_gap() <= 1.0);
                    save("reply-bottom.png");
                    ui.set_settings_open(true);
                    ui.set_tab(0);
                    ui.set_models(ModelRc::new(VecModel::from(
                        (0..30)
                            .map(|n| format!("fixture-model-{n:02}").into())
                            .collect::<Vec<_>>(),
                    )));
                    ui.set_models_message("Preview fixture: click any model to select it.".into());
                }
                4 => {
                    ui.invoke_reveal_models();
                }
                5 => {
                    save("models.png");
                    click(180.0,253.0);
                    assert_eq!(ui.get_model().as_str(),"fixture-model-00");
                    scroll(280.0,300.0);
                }
                6 => {
                    save("models-bottom.png");
                    click(180.0,350.0);
                    assert_eq!(ui.get_model().as_str(),"fixture-model-29");
                    eprintln!("First and last model selection passed.");
                    scroll(465.0,330.0);
                }
                7 => {
                    save("connection-bottom.png");
                    eprintln!("Connection last control gap {}", ui.get_last_control_gap());
                    assert!(ui.get_last_control_gap() <= 0.0);
                    ui.set_tab(2);
                }
                8 => {
                    save("screen-top.png");
                    scroll(280.0, 300.0);
                }
                9 => {
                    save("screen-bottom.png");
                    eprintln!("Screen last control gap {}", ui.get_last_control_gap());
                    assert!(ui.get_last_control_gap() <= 0.0);
                    ui.set_settings_open(false);
                    ui.set_answer("The reply background now extends behind the input bar, forming one continuous panel.\n\nIts opacity is still adjustable in Appearance.\n\nThis is a layout preview.".into());
                }
                10=>{save("joined-panel.png");let _=slint::quit_event_loop();}
                _ => {}
            }
        },
    );
    ui.show()?;
    slint::run_event_loop()?;
    drop(timer);
    Ok(())
}

pub fn settings_update(ui: &InputWindow, directory: String) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    ui.set_settings_open(true);
    ui.set_tab(0);
    let w = ui.as_weak();
    let step = Cell::new(0);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(1200),
        move || {
            let ui = w.upgrade().unwrap();
            let n = step.get();
            step.set(n + 1);
            let save = |name: &str| {
                let frame = ui.window().take_snapshot().unwrap();
                image::save_buffer(
                    Path::new(&directory).join(name),
                    frame.as_bytes(),
                    frame.width(),
                    frame.height(),
                    image::ColorType::Rgba8,
                )
                .unwrap();
            };
            match n {
                0 => {
                    save("connection.png");
                    ui.set_provider_index(1);
                    ui.set_model("claude-fixture".into());
                }
                1 => {
                    save("anthropic.png");
                    ui.set_tab(3);
                }
                2 => {
                    save("shortcuts.png");
                    use slint::platform::{Key, PointerEventButton, WindowEvent};
                    let position = slint::LogicalPosition::new(375.0, 190.0);
                    ui.window().dispatch_event(WindowEvent::PointerPressed {
                        position,
                        button: PointerEventButton::Left,
                    });
                    ui.window().dispatch_event(WindowEvent::PointerReleased {
                        position,
                        button: PointerEventButton::Left,
                    });
                    for text in [Key::Control.into(), Key::Alt.into(), "q".into()] {
                        ui.window().dispatch_event(WindowEvent::KeyPressed { text });
                    }
                    for text in [Key::Control.into(), Key::Alt.into()] {
                        ui.window()
                            .dispatch_event(WindowEvent::KeyReleased { text });
                    }
                    assert_eq!(ui.get_shortcut_window().as_str(), "<Control><Alt>q");
                    eprintln!("Shortcut recording passed.");
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
                            position: slint::LogicalPosition::new(300.0, 300.0),
                            delta_x: 0.0,
                            delta_y: -10000.0,
                        });
                }
                3 => {
                    assert!(ui.get_last_control_gap() <= 0.0);
                    save("shortcuts-bottom.png");
                    let start = ui.window().position();
                    crate::bounds::move_corner(&ui, "br");
                    let weak = ui.as_weak();
                    slint::Timer::single_shot(std::time::Duration::from_millis(120), move || {
                        let ui = weak.upgrade().unwrap();
                        let middle = ui.window().position();

                        eprintln!("Motion intermediate: {:?} -> {:?}", start, middle);
                        let weak = ui.as_weak();
                        slint::Timer::single_shot(
                            std::time::Duration::from_millis(400),
                            move || {
                                let ui = weak.upgrade().unwrap();
                                assert_ne!(start, ui.window().position());
                                eprintln!("Motion final: {:?}", ui.window().position());
                            },
                        );
                    });
                }
                4 => {
                    eprintln!("Corner move position {:?}", ui.window().position());
                    ui.invoke_quit();
                }
                _ => {}
            }
        },
    );
    ui.show()?;
    slint::run_event_loop()?;
    drop(timer);
    Ok(())
}

pub fn focus_and_bounds(ui: &InputWindow) -> anyhow::Result<()> {
    ui.set_settings_open(true);
    let w = ui.as_weak();
    let step = Cell::new(0);
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(1000),
        move || {
            use slint::winit_030::WinitWindowAccessor;
            let ui = w.upgrade().unwrap();
            let n = step.get();
            step.set(n + 1);
            match n {
                0 => {
                    // Exercise the primary laptop display, whose top panel must
                    // be reserved even when _NET_CURRENT_DESKTOP is absent.
                    ui.window().with_winit_window(|native| {
                        if let Some(monitor) = native.primary_monitor() {
                            let p = monitor.position();
                            native.set_outer_position(slint::winit_030::winit::dpi::PhysicalPosition::new(p.x+100,p.y+100));
                            let _ = native.outer_position();
                        }
                    });
                    ui.hide().unwrap();
                    crate::focus_input(&ui);
                }
                1 => {
                    assert!(ui.get_input_focused());
                    assert!(!ui.get_settings_open());
                    ui.window()
                        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
                            text: "focus shortcut works".into(),
                        });
                    assert_eq!(ui.get_question().as_str(), "focus shortcut works");
                    eprintln!("Hidden/settings input focus and typing passed.");
                    crate::bounds::move_corner(&ui, "up");
                }
                2 => {
                    ui.window().with_winit_window(|native| {
                        let m = native.current_monitor().unwrap();
                        let p = m.position();
                        let s = m.size();
                        let r = crate::bounds::WorkArea::new().unwrap().for_monitor(
                            crate::bounds::Rect {
                                x: p.x,
                                y: p.y,
                                w: s.width as i32,
                                h: s.height as i32,
                            },
                        );
                        let actual = native.outer_position().unwrap();
                        assert_eq!(actual.y, r.y);
                        eprintln!("Top edge {:?}, usable area {:?}", actual, r);
                    });
                    crate::bounds::move_corner(&ui, "left");
                }
                3 => {
                    ui.window().with_winit_window(|native| {
                        let m = native.current_monitor().unwrap();
                        let p = m.position();
                        let s = m.size();
                        let r = crate::bounds::WorkArea::new().unwrap().for_monitor(
                            crate::bounds::Rect {
                                x: p.x,
                                y: p.y,
                                w: s.width as i32,
                                h: s.height as i32,
                            },
                        );
                        let actual = native.outer_position().unwrap();
                        assert_eq!((actual.x, actual.y), (r.x, r.y));
                        eprintln!("Left edge has no added outer margin.");
                    });
                    ui.invoke_quit();
                }
                _ => {}
            }
        },
    );
    ui.show()?;
    slint::run_event_loop_until_quit()?;
    drop(timer);
    Ok(())
}
