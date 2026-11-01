//! Keep both expanding layouts and dragged windows inside the desktop work area.
use x11rb::{
    connection::Connection,
    protocol::xproto::{AtomEnum, ConnectionExt},
    rust_connection::RustConnection,
};
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}
pub fn position(area: Rect, x: i32, y: i32, w: i32, h: i32) -> (i32, i32) {
    (
        x.clamp(area.x, area.x + (area.w - w).max(0)),
        y.clamp(area.y, area.y + (area.h - h).max(0)),
    )
}
pub struct WorkArea {
    connection: RustConnection,
    root: u32,
    work: u32,
    desktop: u32,
}
impl WorkArea {
    // GNOME/XWayland can publish per-desktop work areas without publishing
    // _NET_CURRENT_DESKTOP. Desktop zero is the default in that case.
    fn current_desktop(&self) -> u32 {
        let value = (|| {
            self.connection
                .get_property(false, self.root, self.desktop, AtomEnum::CARDINAL, 0, 1)
                .ok()?
                .reply()
                .ok()?
                .value32()?
                .next()
        })();
        desktop_number(value)
    }
    pub fn new() -> Option<Self> {
        let (c, s) = x11rb::connect(None).ok()?;
        let root = c.setup().roots[s].root;
        let work = c
            .intern_atom(false, b"_NET_WORKAREA")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let desktop = c
            .intern_atom(false, b"_NET_CURRENT_DESKTOP")
            .ok()?
            .reply()
            .ok()?
            .atom;
        Some(Self {
            connection: c,
            root,
            work,
            desktop,
        })
    }
    pub fn get(&self) -> Option<Rect> {
        let desktop = self.current_desktop();
        let r = self
            .connection
            .get_property(
                false,
                self.root,
                self.work,
                AtomEnum::CARDINAL,
                desktop * 4,
                4,
            )
            .ok()?
            .reply()
            .ok()?;
        let v: Vec<_> = r.value32()?.collect();
        if v.len() != 4 {
            return None;
        }
        Some(Rect {
            x: v[0] as i32,
            y: v[1] as i32,
            w: v[2] as i32,
            h: v[3] as i32,
        })
    }
}
fn desktop_number(value: Option<u32>) -> u32 {
    value.unwrap_or(0)
}
pub fn monitor_work_area(monitor: Rect, areas: &[Rect]) -> Rect {
    areas
        .iter()
        .filter_map(|area| {
            let x = monitor.x.max(area.x);
            let y = monitor.y.max(area.y);
            let right = (monitor.x + monitor.w).min(area.x + area.w);
            let bottom = (monitor.y + monitor.h).min(area.y + area.h);
            (right > x && bottom > y).then_some(Rect {
                x,
                y,
                w: right - x,
                h: bottom - y,
            })
        })
        .max_by_key(|a| a.w as i64 * a.h as i64)
        .unwrap_or(monitor)
}
impl WorkArea {
    pub fn for_monitor(&self, monitor: Rect) -> Rect {
        let gtk = (|| -> Option<Vec<Rect>> {
            let desktop = self.current_desktop();
            let name = format!("_GTK_WORKAREAS_D{desktop}");
            let atom = self
                .connection
                .intern_atom(true, name.as_bytes())
                .ok()?
                .reply()
                .ok()?
                .atom;
            if atom == 0 {
                return None;
            }
            let values = self
                .connection
                .get_property(false, self.root, atom, AtomEnum::CARDINAL, 0, 4096)
                .ok()?
                .reply()
                .ok()?;
            let data: Vec<u32> = values.value32()?.collect();
            if data.is_empty() || data.len() % 4 != 0 {
                return None;
            }
            Some(
                data.chunks_exact(4)
                    .map(|v| Rect {
                        x: v[0] as i32,
                        y: v[1] as i32,
                        w: v[2] as i32,
                        h: v[3] as i32,
                    })
                    .collect(),
            )
        })();
        if let Some(areas) = gtk {
            return monitor_work_area(monitor, &areas);
        }
        monitor_work_area(monitor, &self.get().into_iter().collect::<Vec<_>>())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clamps_resize_and_negative_monitor() {
        let r = Rect {
            x: -1920,
            y: 32,
            w: 1920,
            h: 1048,
        };
        assert_eq!(position(r, -100, 900, 500, 500), (-500, 580));
        assert_eq!(position(r, -3000, -20, 500, 500), (-1920, 32));
    }
    #[test]
    fn oversized_windows_do_not_invert_range() {
        assert_eq!(
            position(
                Rect {
                    x: 0,
                    y: 40,
                    w: 400,
                    h: 300
                },
                10,
                10,
                500,
                500
            ),
            (0, 40)
        );
    }
}

thread_local! {static MOVEMENT:std::cell::RefCell<Option<slint::Timer>>=const {std::cell::RefCell::new(None)};}
pub fn cancel_animation() {
    GOAL.with(|goal| goal.set(None));
    MOVEMENT.with(|timer| {
        if let Some(timer) = timer.borrow().as_ref() {
            timer.stop();
        }
    });
}
pub fn directional_target(
    area: Rect,
    current: (i32, i32),
    size: (i32, i32),
    direction: &str,
) -> Option<(i32, i32)> {
    let left = area.x;
    let right = area.x + area.w - size.0;
    let top = area.y;
    let bottom = area.y + area.h - size.1;
    let target = match direction {
        "left" => (left, current.1),
        "right" => (right, current.1),
        "up" => (current.0, top),
        "down" => (current.0, bottom),
        "tl" => (left, top),
        "tr" => (right, top),
        "bl" => (left, bottom),
        "br" => (right, bottom),
        _ => return None,
    };
    Some(position(area, target.0, target.1, size.0, size.1))
}
thread_local! {static GOAL:std::cell::Cell<Option<(i32,i32)>>=const{std::cell::Cell::new(None)};}
pub fn move_corner(ui: &crate::InputWindow, corner: &str) {
    let pending = GOAL.with(|goal| goal.get());
    cancel_animation();
    use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
    let destination = ui
        .window()
        .with_winit_window(|native| {
            let Some(m) = native
                .current_monitor()
                .or_else(|| native.primary_monitor())
            else {
                return None;
            };
            let p = m.position();
            let size = m.size();
            let mut r = Rect {
                x: p.x,
                y: p.y,
                w: size.width as i32,
                h: size.height as i32,
            };
            if let Some(work) = WorkArea::new() {
                r = work.for_monitor(r);
            }
            let size = native.outer_size();
            let current = native.outer_position().ok()?;
            let start = (current.x, current.y);
            let target = directional_target(
                r,
                pending.unwrap_or(start),
                (size.width as i32, size.height as i32),
                corner,
            )?;
            Some((start, target))
        })
        .flatten();
    let Some((start, end)) = destination else {
        return;
    };
    if start == end {
        return;
    }
    GOAL.with(|goal| goal.set(Some(end)));
    let weak = ui.as_weak();
    let began = std::time::Instant::now();
    let trace = std::env::var_os("OPTIC_CHAT_SETTINGS_CHECK_DIR").is_some();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(16),
        move || {
            let Some(ui) = weak.upgrade() else {
                cancel_animation();
                return;
            };
            let t = (began.elapsed().as_secs_f64() / 0.32).min(1.0);
            let eased = t * t * (3.0 - 2.0 * t);
            let x = (start.0 as f64 + (end.0 - start.0) as f64 * eased).round() as i32;
            let y = (start.1 as f64 + (end.1 - start.1) as f64 * eased).round() as i32;
            ui.window().with_winit_window(|native| {
                native
                    .set_outer_position(slint::winit_030::winit::dpi::PhysicalPosition::new(x, y));
                // Flush X11's queued configure request each frame, even when the UI is idle.
                let actual = native.outer_position();
                if trace {
                    eprintln!("Motion frame {t:.3}: {actual:?}");
                }
            });
            if t >= 1.0 {
                cancel_animation();
            }
        },
    );
    MOVEMENT.with(|stored| *stored.borrow_mut() = Some(timer));
}

#[cfg(test)]
mod direction_tests {
    use super::*;
    #[test]
    fn arrows_preserve_the_other_axis() {
        let r = Rect {
            x: 0,
            y: 30,
            w: 1920,
            h: 1050,
        };
        let size = (500, 100);
        for y in [30, 450, 980] {
            assert_eq!(directional_target(r, (1420, y), size, "left"), Some((0, y)));
            assert_eq!(
                directional_target(r, (0, y), size, "right"),
                Some((1420, y))
            );
        }
        for x in [0, 750, 1420] {
            assert_eq!(directional_target(r, (x, 980), size, "up"), Some((x, 30)));
            assert_eq!(directional_target(r, (x, 30), size, "down"), Some((x, 980)));
        }
        assert!(directional_target(r, (0, 0), size, "invalid").is_none());
    }
}

#[cfg(test)]
mod workarea_tests {
    use super::*;
    #[test]
    fn missing_current_desktop_still_uses_desktop_zero_work_area() {
        assert_eq!(desktop_number(None), 0);
        assert_eq!(desktop_number(Some(2)), 2);
        let monitor = Rect { x:0, y:1080, w:1920, h:1080 };
        let areas = [Rect { x:0, y:1112, w:1920, h:1048 }];
        let safe = monitor_work_area(monitor, &areas);
        assert_eq!(directional_target(safe, (600,1800), (500,100), "up"), Some((600,1112)));
    }
    #[test]
    fn reserves_panel_only_on_its_monitor() {
        let areas = [
            Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080,
            },
            Rect {
                x: 0,
                y: 1112,
                w: 1920,
                h: 1048,
            },
        ];
        assert_eq!(
            monitor_work_area(
                Rect {
                    x: 0,
                    y: 1080,
                    w: 1920,
                    h: 1080
                },
                &areas
            ),
            areas[1]
        );
        assert_eq!(monitor_work_area(areas[0], &areas), areas[0]);
    }
    #[test]
    fn no_panel_does_not_add_a_gap() {
        let monitor = Rect {
            x: -1920,
            y: 0,
            w: 1920,
            h: 1080,
        };
        assert_eq!(monitor_work_area(monitor, &[]), monitor);
        assert_eq!(
            directional_target(monitor, (-1000, 500), (500, 100), "up"),
            Some((-1000, 0))
        );
    }
}
