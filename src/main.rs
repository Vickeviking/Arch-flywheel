//! Arch-flywheel — en GTA-stil radial meny för Hyprland.
//!
//! Tryck SUPER+SPACE och **släpp** (håll inte — annars blir varje tangent en
//! SUPER-chord som Hyprlands globala bindningar fångar). Navigera sedan med
//! musen eller J/K (H/L för in/ut i undermenyer). SPACE/Enter/vänsterklick
//! väljer, H/Backspace/högerklick backar, Esc stänger.

use std::cell::RefCell;
use std::process::Command;
use std::rc::Rc;

use gtk4::cairo::{Context, FontSlant, FontWeight};
use gtk4::gdk::Key;
use gtk4::prelude::*;
use gtk4::{
    glib, Application, ApplicationWindow, CssProvider, DrawingArea, EventControllerKey,
    EventControllerMotion, GestureClick,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::Deserialize;

const APP_ID: &str = "dev.vickeviking.arch_flywheel";
/// Rust-orange (#CE422B).
const ACCENT: (f64, f64, f64) = (0xCE as f64 / 255.0, 0x42 as f64 / 255.0, 0x2B as f64 / 255.0);
/// Övre tak på hjulets radie (px) så det inte blir enormt på stora skärmar.
const MAX_OUTER: f64 = 250.0;

#[derive(Deserialize, Clone, Debug, Default)]
struct Item {
    label: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    cmd: Option<String>,
    #[serde(default)]
    items: Vec<Item>,
}

impl Item {
    fn is_menu(&self) -> bool {
        !self.items.is_empty()
    }
}

#[derive(Deserialize, Default)]
struct Menu {
    #[serde(default)]
    items: Vec<Item>,
}

struct State {
    stack: Vec<Vec<Item>>,
    sel: usize,
    size: (i32, i32),
}

impl State {
    fn current(&self) -> &[Item] {
        self.stack.last().map(|v| v.as_slice()).unwrap_or(&[])
    }
}

fn config_path() -> std::path::PathBuf {
    let base = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    std::path::Path::new(&base)
        .join("arch-flywheel")
        .join("menu.toml")
}

/// Läs menyn: användarens fil om den finns, annars den inbäddade standarden
/// (som också skrivs till configsökvägen första gången så den går att redigera).
fn load_menu() -> Vec<Item> {
    let default = include_str!("../menu.toml");
    let path = config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&path, default);
            default.to_string()
        }
    };
    let parsed = toml::from_str::<Menu>(&text)
        .ok()
        .filter(|m| !m.items.is_empty())
        .or_else(|| toml::from_str::<Menu>(default).ok());
    parsed.map(|m| m.items).unwrap_or_default()
}

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn build_ui(app: &Application) {
    let items = load_menu();
    let state = Rc::new(RefCell::new(State {
        stack: vec![items],
        sel: 0,
        size: (0, 0),
    }));

    let window = ApplicationWindow::builder().application(app).build();
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_namespace("arch-flywheel");
    for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
        window.set_anchor(edge, true);
    }

    // Genomskinligt fönster så skärmen lyser igenom bakom skrim-lagret.
    let provider = CssProvider::new();
    provider.load_from_data("window.flywheel { background-color: transparent; }");
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    window.add_css_class("flywheel");

    let area = DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    {
        let state = state.clone();
        area.set_draw_func(move |_, cr, w, h| {
            state.borrow_mut().size = (w, h);
            draw(cr, &state.borrow());
        });
    }
    window.set_child(Some(&area));

    // Tangentbord — INGEN SUPER-hållning: vanliga tangenter navigerar.
    let keyc = EventControllerKey::new();
    {
        let state = state.clone();
        let area = area.clone();
        let app = app.clone();
        keyc.connect_key_pressed(move |_, key, _code, _mods| {
            let mut quit = false;
            {
                let mut s = state.borrow_mut();
                let n = s.current().len();
                match key {
                    Key::j | Key::Down => {
                        if n > 0 {
                            s.sel = (s.sel + 1) % n;
                        }
                    }
                    Key::k | Key::Up => {
                        if n > 0 {
                            s.sel = (s.sel + n - 1) % n;
                        }
                    }
                    Key::l | Key::Right | Key::space | Key::Return | Key::KP_Enter => {
                        quit = activate(&mut s);
                    }
                    Key::h | Key::Left | Key::BackSpace => back(&mut s),
                    Key::Escape | Key::q => quit = true,
                    _ => {}
                }
            }
            if quit {
                app.quit();
            } else {
                area.queue_draw();
            }
            glib::Propagation::Stop
        });
    }
    window.add_controller(keyc);

    // Mus: rörelse markerar, vänsterklick väljer, högerklick backar.
    let motion = EventControllerMotion::new();
    {
        let state = state.clone();
        let area = area.clone();
        motion.connect_motion(move |_, x, y| {
            {
                let mut s = state.borrow_mut();
                if let Some(sel) = sector_at(&s, x, y) {
                    s.sel = sel;
                }
            }
            area.queue_draw();
        });
    }
    area.add_controller(motion);

    let left = GestureClick::new();
    left.set_button(1);
    {
        let state = state.clone();
        let app = app.clone();
        let area = area.clone();
        left.connect_pressed(move |_, _, x, y| {
            let quit = {
                let mut s = state.borrow_mut();
                match sector_at(&s, x, y) {
                    Some(sel) => {
                        s.sel = sel;
                        activate(&mut s)
                    }
                    None => true, // klick i mitten = stäng
                }
            };
            if quit {
                app.quit();
            } else {
                area.queue_draw();
            }
        });
    }
    area.add_controller(left);

    let right = GestureClick::new();
    right.set_button(3);
    {
        let state = state.clone();
        let app = app.clone();
        let area = area.clone();
        right.connect_pressed(move |_, _, _, _| {
            let close = {
                let mut s = state.borrow_mut();
                if s.stack.len() > 1 {
                    back(&mut s);
                    false
                } else {
                    true
                }
            };
            if close {
                app.quit();
            } else {
                area.queue_draw();
            }
        });
    }
    area.add_controller(right);

    window.present();
}

/// Returnerar true om appen ska avslutas (ett blad öppnades).
fn activate(s: &mut State) -> bool {
    let item = match s.current().get(s.sel) {
        Some(it) => it.clone(),
        None => return true,
    };
    if item.is_menu() {
        s.stack.push(item.items);
        s.sel = 0;
        false
    } else {
        run_action(&item);
        true
    }
}

fn back(s: &mut State) {
    if s.stack.len() > 1 {
        s.stack.pop();
        s.sel = 0;
    }
}

fn run_action(item: &Item) {
    if let Some(url) = &item.url {
        let _ = Command::new("xdg-open").arg(url).spawn();
    } else if let Some(cmd) = &item.cmd {
        let _ = Command::new("sh").arg("-c").arg(cmd).spawn();
    }
}

fn ring_outer(w: i32, h: i32) -> f64 {
    ((w.min(h) as f64) * 0.26).min(MAX_OUTER)
}

fn ring_inner(w: i32, h: i32) -> f64 {
    ring_outer(w, h) * 0.46
}

/// Vilken sektor pekaren står över (None = dödzon i mitten).
fn sector_at(s: &State, x: f64, y: f64) -> Option<usize> {
    let (w, h) = s.size;
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let (dx, dy) = (x - cx, y - cy);
    let dist = (dx * dx + dy * dy).sqrt();
    let n = s.current().len();
    if n == 0 || dist < ring_inner(w, h) * 0.75 {
        return None;
    }
    let tau = std::f64::consts::TAU;
    let mut ang = dy.atan2(dx) + std::f64::consts::FRAC_PI_2;
    ang = ((ang % tau) + tau) % tau;
    let seg = tau / n as f64;
    let idx = (((ang + seg / 2.0) % tau) / seg).floor() as usize % n;
    Some(idx)
}

/// Centrera text i (x, y) med hjälp av cairo-bearings.
fn centered_text(cr: &Context, text: &str, x: f64, y: f64) {
    match cr.text_extents(text) {
        Ok(e) => cr.move_to(x - e.width() / 2.0 - e.x_bearing(), y - e.height() / 2.0 - e.y_bearing()),
        Err(_) => cr.move_to(x, y),
    }
}

fn draw(cr: &Context, s: &State) {
    let (w, h) = s.size;
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);

    // Mörkt skrim.
    cr.set_source_rgba(0.03, 0.03, 0.045, 0.45);
    let _ = cr.paint();

    let items = s.current();
    let n = items.len();
    if n == 0 {
        return;
    }
    let outer = ring_outer(w, h);
    let inner = ring_inner(w, h);
    let tau = std::f64::consts::TAU;
    let seg = tau / n as f64;
    let gap = (0.045_f64).min(seg * 0.16); // lucka mellan kilar

    cr.select_font_face("sans-serif", FontSlant::Normal, FontWeight::Bold);

    for (i, item) in items.iter().enumerate() {
        let base = -std::f64::consts::FRAC_PI_2 - seg / 2.0 + seg * i as f64;
        let a0 = base + gap / 2.0;
        let a1 = base + seg - gap / 2.0;
        let hot = i == s.sel;

        cr.new_path();
        cr.arc(cx, cy, outer, a0, a1);
        cr.arc_negative(cx, cy, inner, a1, a0);
        cr.close_path();
        if hot {
            cr.set_source_rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.96);
        } else {
            cr.set_source_rgba(0.13, 0.14, 0.17, 0.94);
        }
        let _ = cr.fill();

        let mid = (a0 + a1) / 2.0;
        let rr = (outer + inner) / 2.0;
        let lx = cx + mid.cos() * rr;
        let ly = cy + mid.sin() * rr;
        cr.set_font_size(14.5);
        centered_text(cr, &item.label, lx, ly);
        if hot {
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        } else {
            cr.set_source_rgba(0.80, 0.82, 0.87, 1.0);
        }
        let _ = cr.show_text(&item.label);

        if item.is_menu() {
            let ax = cx + mid.cos() * (outer - 11.0);
            let ay = cy + mid.sin() * (outer - 11.0);
            cr.set_font_size(12.0);
            centered_text(cr, "›", ax, ay);
            if hot {
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            } else {
                cr.set_source_rgba(ACCENT.0 + 0.25, ACCENT.1 + 0.2, ACCENT.2 + 0.15, 1.0);
            }
            let _ = cr.show_text("›");
        }
    }

    // Nav i mitten.
    cr.new_path();
    cr.arc(cx, cy, inner - 8.0, 0.0, tau);
    cr.set_source_rgba(0.07, 0.08, 0.10, 0.98);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.55);
    cr.set_line_width(2.0);
    let _ = cr.stroke();

    if let Some(item) = items.get(s.sel) {
        cr.select_font_face("sans-serif", FontSlant::Normal, FontWeight::Bold);
        cr.set_font_size(14.0);
        centered_text(cr, &item.label, cx, cy - 7.0);
        cr.set_source_rgba(0.98, 0.66, 0.46, 1.0);
        let _ = cr.show_text(&item.label);

        let hint = if item.is_menu() { "öppna ›" } else { "Enter" };
        cr.select_font_face("sans-serif", FontSlant::Normal, FontWeight::Normal);
        cr.set_font_size(10.0);
        centered_text(cr, hint, cx, cy + 11.0);
        cr.set_source_rgba(0.6, 0.62, 0.68, 1.0);
        let _ = cr.show_text(hint);
    }
}
