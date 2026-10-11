//! Arch-flywheel — en GTA-stil radial meny för Hyprland.
//!
//! Håll SUPER+SPACE → hjulet upp. Navigera med musen eller J/K. SPACE/L går in
//! i en kategori eller öppnar ett blad, H/Backspace backar. Släpp SUPER för att
//! öppna det markerade bladet (eller avbryt om du står på en kategori). Esc
//! avbryter alltid.

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
    let base = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| {
        format!("{}/.config", std::env::var("HOME").unwrap_or_default())
    });
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

    // Gör fönstret genomskinligt så skärmen lyser igenom bakom skrim-lagret.
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

    // Tangentbord: tryck navigerar, släpp av SUPER bekräftar.
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
                    Key::l | Key::Right | Key::space | Key::Return => {
                        quit = activate(&mut s);
                    }
                    Key::h | Key::Left | Key::BackSpace => back(&mut s),
                    Key::Escape => quit = true,
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
    {
        let state = state.clone();
        let app = app.clone();
        keyc.connect_key_released(move |_, key, _code, _mods| {
            if matches!(key, Key::Super_L | Key::Super_R) {
                {
                    let mut s = state.borrow_mut();
                    let is_leaf = s
                        .current()
                        .get(s.sel)
                        .map(|it| !it.is_menu())
                        .unwrap_or(false);
                    if is_leaf {
                        activate(&mut s);
                    }
                }
                app.quit();
            }
        });
    }
    window.add_controller(keyc);

    // Mus: rörelse markerar sektorn, vänsterklick aktiverar.
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

    let click = GestureClick::new();
    {
        let state = state.clone();
        let app = app.clone();
        let area = area.clone();
        click.connect_pressed(move |_, _, x, y| {
            let quit = {
                let mut s = state.borrow_mut();
                match sector_at(&s, x, y) {
                    Some(sel) => {
                        s.sel = sel;
                        activate(&mut s)
                    }
                    None => true, // klick i mitten = avbryt
                }
            };
            if quit {
                app.quit();
            } else {
                area.queue_draw();
            }
        });
    }
    area.add_controller(click);

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
    (w.min(h) as f64) * 0.34
}

fn ring_inner(w: i32, h: i32) -> f64 {
    (w.min(h) as f64) * 0.16
}

/// Vilken sektor pekaren står över (None = dödzon i mitten).
fn sector_at(s: &State, x: f64, y: f64) -> Option<usize> {
    let (w, h) = s.size;
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
    let (dx, dy) = (x - cx, y - cy);
    let dist = (dx * dx + dy * dy).sqrt();
    let n = s.current().len();
    if n == 0 || dist < ring_inner(w, h) * 0.7 {
        return None;
    }
    let tau = std::f64::consts::TAU;
    // 0 rad = rakt upp, medurs.
    let mut ang = dy.atan2(dx) + std::f64::consts::FRAC_PI_2;
    ang = ((ang % tau) + tau) % tau;
    let seg = tau / n as f64;
    let idx = (((ang + seg / 2.0) % tau) / seg).floor() as usize % n;
    Some(idx)
}

fn draw(cr: &Context, s: &State) {
    let (w, h) = s.size;
    let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);

    // Mörkt skrim över skärmen.
    cr.set_source_rgba(0.04, 0.04, 0.05, 0.55);
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

    for (i, item) in items.iter().enumerate() {
        let a0 = -std::f64::consts::FRAC_PI_2 - seg / 2.0 + seg * i as f64;
        let a1 = a0 + seg;
        let hot = i == s.sel;

        cr.new_path();
        cr.arc(cx, cy, outer, a0, a1);
        cr.arc_negative(cx, cy, inner, a1, a0);
        cr.close_path();
        if hot {
            cr.set_source_rgba(ACCENT.0, ACCENT.1, ACCENT.2, 0.93);
        } else {
            cr.set_source_rgba(0.12, 0.13, 0.16, 0.90);
        }
        let _ = cr.fill_preserve();
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.5);
        cr.set_line_width(2.0);
        let _ = cr.stroke();

        let mid = (a0 + a1) / 2.0;
        let rr = (outer + inner) / 2.0;
        let lx = cx + mid.cos() * rr;
        let ly = cy + mid.sin() * rr;
        let marker = if item.is_menu() { " ›" } else { "" };
        let label = format!("{}{}", item.label, marker);
        cr.select_font_face("sans-serif", FontSlant::Normal, FontWeight::Bold);
        cr.set_font_size(18.0);
        match cr.text_extents(&label) {
            Ok(ext) => cr.move_to(lx - ext.width() / 2.0, ly + ext.height() / 2.0),
            Err(_) => cr.move_to(lx, ly),
        }
        if hot {
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        } else {
            cr.set_source_rgba(0.85, 0.86, 0.90, 1.0);
        }
        let _ = cr.show_text(&label);
    }

    // Nav i mitten.
    cr.arc(cx, cy, inner * 0.95, 0.0, tau);
    cr.set_source_rgba(0.08, 0.09, 0.11, 0.96);
    let _ = cr.fill();

    let depth = s.stack.len();
    let title = if depth > 1 { "▸" } else { "🦀" };
    cr.select_font_face("sans-serif", FontSlant::Normal, FontWeight::Normal);
    cr.set_font_size(30.0);
    if let Ok(ext) = cr.text_extents(title) {
        cr.move_to(cx - ext.width() / 2.0, cy - 2.0);
    }
    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    let _ = cr.show_text(title);

    if let Some(item) = items.get(s.sel) {
        cr.set_font_size(14.0);
        if let Ok(ext) = cr.text_extents(&item.label) {
            cr.move_to(cx - ext.width() / 2.0, cy + 26.0);
        }
        cr.set_source_rgba(0.95, 0.60, 0.42, 1.0);
        let _ = cr.show_text(&item.label);
    }
}
