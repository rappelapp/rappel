//! Throwaway spike: can RaTeX render Cards' formulas in GTK4?
//!
//!   cargo run --release                      window: inline A/B + display
//!   cargo run --release -- --dark            start in dark mode
//!   cargo run --release -- --width 360       start 360 px wide
//!   cargo run --release -- --count 50        a "card" with 50 display formulas
//!   cargo run --release -- --screenshot F.png   render the page to a PNG and quit
//!   cargo run --release -- --bench           headless timings
//!   cargo run --release -- --errors          invalid LaTeX handling (headless)

mod formulas;
mod inline;
mod math;

use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gdk, glib, graphene};
use ratex_types::Color;

/// KaTeX sets `.katex { font-size: 1.21em }`: math is 21 % bigger than text.
pub const MATH_SCALE: f64 = 1.21;

#[derive(Debug, Default, Clone)]
struct Args {
    dark: bool,
    width: i32,
    count: usize,
    screenshot: Option<String>,
}

fn main() -> glib::ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.iter().any(|a| a == "--bench") {
        bench();
        return glib::ExitCode::SUCCESS;
    }
    if raw.iter().any(|a| a == "--side") {
        side();
        return glib::ExitCode::SUCCESS;
    }
    if raw.iter().any(|a| a == "--fallback") {
        fallback();
        return glib::ExitCode::SUCCESS;
    }
    if raw.iter().any(|a| a == "--errors") {
        errors();
        return glib::ExitCode::SUCCESS;
    }
    if let Some(i) = raw.iter().position(|a| a == "--error-one") {
        error_one(raw[i + 1].parse().unwrap());
        return glib::ExitCode::SUCCESS;
    }
    let mut args = Args {
        width: 760,
        ..Args::default()
    };
    let mut it = raw.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dark" => args.dark = true,
            "--width" => args.width = it.next().unwrap().parse().unwrap(),
            "--count" => args.count = it.next().unwrap().parse().unwrap(),
            "--screenshot" => args.screenshot = it.next().cloned(),
            other => panic!("unknown arg {other}"),
        }
    }
    let app = adw::Application::builder()
        .application_id("io.github.rsmirke.Cards.MathSpike")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| build(app, &args));
    app.run_with_args::<&str>(&[])
}

/// Body text size in px, from the widget's Pango font (points at 96 dpi).
fn em_px(w: &impl IsA<gtk::Widget>) -> f64 {
    let desc = w.pango_context().font_description().unwrap();
    let size = f64::from(desc.size()) / f64::from(gtk::pango::SCALE);
    if desc.is_size_absolute() {
        size
    } else {
        size * 96.0 / 72.0
    }
}

fn heading(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .margin_top(12)
        .build();
    l.add_css_class("heading");
    l
}

fn dim(text: &str) -> gtk::Label {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    l.add_css_class("dim-label");
    l.add_css_class("caption");
    l
}

fn display_formula(src: &str, em: f64) -> gtk::Widget {
    let f = ratex_gtk4::RatexFormula::new();
    f.set_display_mode(true);
    f.set_font_size(em * MATH_SCALE);
    f.set_padding(4.0);
    f.set_latex(src);
    f.set_halign(gtk::Align::Center);
    if let Some(err) = f.error_message() {
        let l = gtk::Label::new(Some(&format!("{src}\n⚠ {err}")));
        l.add_css_class("monospace");
        l.add_css_class("error");
        l.set_wrap(true);
        return l.upcast();
    }
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&f)
        .build()
        .upcast()
}

/// Inline A: a TextView with a RatexFormula child per formula.
fn inline_a(sentence: &str, em: f64) -> gtk::TextView {
    let tv = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .wrap_mode(gtk::WrapMode::WordChar)
        .build();
    tv.add_css_class("inline-a");
    let buf = tv.buffer();
    for (is_math, piece) in formulas::split(sentence) {
        let mut end = buf.end_iter();
        if is_math {
            let anchor = buf.create_child_anchor(&mut end);
            let f = ratex_gtk4::RatexFormula::new();
            f.set_display_mode(false);
            f.set_font_size(em * MATH_SCALE);
            f.set_padding(0.0);
            f.set_latex(piece);
            tv.add_child_at_anchor(&f, &anchor);
        } else {
            buf.insert(&mut end, piece);
        }
    }
    tv
}

fn build(app: &adw::Application, args: &Args) {
    let sm = adw::StyleManager::default();
    sm.set_color_scheme(if args.dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });

    let provider = gtk::CssProvider::new();
    provider.load_from_string("textview.inline-a, textview.inline-a text { background: none; }");
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(12);
    content.set_margin_bottom(24);
    let em = em_px(&content);

    let built = Instant::now();
    if args.count > 0 {
        content.append(&heading(&format!("{} display formulas", args.count)));
        for (i, (_, src, _)) in formulas::all().iter().cycle().take(args.count).enumerate() {
            content.append(&dim(&format!("{}", i + 1)));
            content.append(&display_formula(src, em));
        }
    } else {
        content.append(&dim(&format!(
            "RaTeX spike · body {em:.2} px · math {:.2} px",
            em * MATH_SCALE
        )));
        content.append(&heading("Inline B — Pango AttrShape (baseline-aligned)"));
        for s in formulas::INLINE {
            let pieces = formulas::split(s)
                .into_iter()
                .map(|(m, p)| (m, p.to_string()))
                .collect();
            content.append(&inline::InlineMath::new(pieces, em));
        }
        content.append(&heading("Inline A — TextView child anchors"));
        for s in formulas::INLINE {
            content.append(&inline_a(s, em));
        }
        content.append(&heading("Display"));
        for (i, src) in formulas::DISPLAY.iter().enumerate() {
            content.append(&dim(&format!("{}", i + 5)));
            content.append(&display_formula(src, em));
        }
        content.append(&heading("Invalid LaTeX"));
        for src in formulas::INVALID {
            content.append(&display_formula(src, em));
        }
    }
    let build_time = built.elapsed();

    let clamp = adw::Clamp::builder()
        .maximum_size(720)
        .child(&content)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let dark = gtk::ToggleButton::builder()
        .icon_name("weather-clear-night-symbolic")
        .tooltip_text("Dark mode")
        .active(args.dark)
        .build();
    dark.connect_toggled(|b| {
        adw::StyleManager::default().set_color_scheme(if b.is_active() {
            adw::ColorScheme::ForceDark
        } else {
            adw::ColorScheme::ForceLight
        });
    });
    header.pack_start(&dark);
    toolbar.add_top_bar(&header);

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("RaTeX spike")
        .default_width(args.width)
        .default_height(900)
        .content(&toolbar)
        .build();

    if args.screenshot.is_some() {
        // No scrolling: the window takes the page's natural height.
        toolbar.set_content(Some(&clamp));
        window.set_default_height(-1);
    } else {
        let sw = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .build();
        toolbar.set_content(Some(&sw));
    }

    let shown = Instant::now();
    let path = args.screenshot.clone();
    let count = args.count;
    window.present();
    let clock_widget = content.clone();
    let once = std::cell::Cell::new(false);
    content.add_tick_callback(move |_, clock| {
        if !once.replace(true) {
            let painted = std::cell::Cell::new(false);
            clock.connect_after_paint(move |_| {
                if count > 0 && !painted.replace(true) {
                    eprintln!("{count} formulas: first frame painted {:?} after present", shown.elapsed());
                }
            });
            if count > 0 {
                eprintln!("{count} formulas: widgets built in {build_time:?}, first tick after present {:?}", shown.elapsed());
            }
            if let Some(path) = path.clone() {
                let w = clock_widget.clone();
                // A WidgetPaintable only holds an image once the widget has
                // been drawn while the paintable existed, so create it now and
                // force a redraw before taking the picture.
                let target: gtk::Widget = w.root().and_downcast::<adw::ApplicationWindow>().unwrap().content().unwrap();
                let paintable = gtk::WidgetPaintable::new(Some(&target));
                target.queue_draw();
                glib::timeout_add_local_once(Duration::from_millis(600), move || {
                    screenshot(&target, &paintable, &path);
                    target.root().and_downcast::<gtk::Window>().unwrap().close();
                });
            }
        }
        glib::ControlFlow::Continue
    });
}

fn screenshot(target: &gtk::Widget, paintable: &gtk::WidgetPaintable, path: &str) {
    let (w, h) = (target.width() as f32, target.height() as f32);
    // Broadway ignores GDK_SCALE; SPIKE_SCALE replays the (vector) nodes at a
    // higher device scale, like a HiDPI monitor would.
    let sf = std::env::var("SPIKE_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(target.scale_factor() as f32);
    let snap = gtk::Snapshot::new();
    snap.scale(sf, sf);
    let bg = if adw::StyleManager::default().is_dark() {
        gdk::RGBA::new(
            0x22 as f32 / 255.0,
            0x22 as f32 / 255.0,
            0x26 as f32 / 255.0,
            1.0,
        )
    } else {
        gdk::RGBA::new(0.98, 0.98, 0.98, 1.0)
    };
    snap.append_color(&bg, &graphene::Rect::new(0.0, 0.0, w, h));
    paintable.snapshot(&snap, f64::from(w), f64::from(h));
    let node = snap.to_node().unwrap();
    // Broadway's own renderer cannot render offscreen; a software cairo
    // renderer realized without a surface can.
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize(None).unwrap();
    let tex = renderer.render_texture(&node, Some(&graphene::Rect::new(0.0, 0.0, w * sf, h * sf)));
    renderer.unrealize();
    tex.save_to_png(path).unwrap();
    eprintln!(
        "wrote {path} ({}x{} px, scale {sf})",
        tex.width(),
        tex.height()
    );
}

fn stats(mut v: Vec<Duration>) -> String {
    v.sort();
    let total: Duration = v.iter().sum();
    let p = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    format!(
        "total {total:>10.2?}  median {:>9.2?}  p95 {:>9.2?}  max {:>9.2?}",
        p(0.5),
        p(0.95),
        p(1.0)
    )
}

/// Headless: parse+layout and cairo render (to an image at scale 2) for a
/// card of 50 formulas, cold (first pass, fonts load) then warm.
fn bench() {
    let all = formulas::all();
    let card: Vec<_> = all.iter().cycle().take(50).collect();
    let font_size = 16.0 * MATH_SCALE;
    for pass in ["cold", "warm", "warm"] {
        let (mut lay, mut ren) = (Vec::new(), Vec::new());
        for (_, src, display) in &card {
            let t = Instant::now();
            let dl = math::display_list(src, *display, Color::BLACK).unwrap();
            lay.push(t.elapsed());
            let m = math::metrics(&dl, font_size, 4.0);
            let t = Instant::now();
            let surf = cairo::ImageSurface::create(
                cairo::Format::ARgb32,
                (m.width * 2.0).ceil() as i32,
                (m.total_height * 2.0).ceil() as i32,
            )
            .unwrap();
            let cr = cairo::Context::new(&surf).unwrap();
            cr.scale(2.0, 2.0);
            ratex_cairo::render_to_cairo(&cr, &dl, &math::options(font_size, 4.0)).unwrap();
            drop(cr);
            surf.flush();
            ren.push(t.elapsed());
        }
        println!("{pass}  parse+layout: {}", stats(lay));
        println!("{pass}  render     : {}", stats(ren));
    }
    for (label, src, display) in &all {
        let t = Instant::now();
        let n = 20;
        for _ in 0..n {
            let dl = math::display_list(src, *display, Color::BLACK).unwrap();
            let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, 2000, 400).unwrap();
            let cr = cairo::Context::new(&surf).unwrap();
            ratex_cairo::render_to_cairo(&cr, &dl, &math::options(font_size, 4.0)).unwrap();
        }
        println!(
            "formula {label:>3}: {:>9.2?} each (layout + render)",
            t.elapsed() / n
        );
    }
}

fn hostile() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = formulas::INVALID
        .iter()
        .map(|s| (s.to_string(), s.to_string()))
        .collect();
    v.push(("1 MiB of '{'".into(), "{".repeat(1 << 20)));
    v.push((
        "10k nested \\frac".into(),
        r"\frac{".repeat(10_000) + &"}{x}".repeat(10_000),
    ));
    v.push((
        "2k nested braces {{…x…}}".into(),
        "{".repeat(2000) + "x" + &"}".repeat(2000),
    ));
    v.push((
        "200 nested \\sqrt".into(),
        r"\sqrt{".repeat(200) + "x" + &"}".repeat(200),
    ));
    v.push(("100k x's".into(), "x".repeat(100_000)));
    v.push((
        "\\rule{1e6em}{1e6em}".into(),
        r"\rule{1000000em}{1000000em}".into(),
    ));
    v.push((
        "\\href (network?)".into(),
        r"\href{https://example.com}{x}".into(),
    ));
    v.push((
        "\\includegraphics".into(),
        r"\includegraphics{/etc/passwd}".into(),
    ));
    v.push(("\\url".into(), r"\url{https://example.com}".into()));
    v
}

fn errors() {
    let exe = std::env::current_exe().unwrap();
    for (i, (label, _)) in hostile().iter().enumerate() {
        let t = Instant::now();
        let out = std::process::Command::new(&exe)
            .args(["--error-one", &i.to_string()])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let status = if out.status.success() {
            "ok".to_string()
        } else {
            format!("CRASH {}", out.status)
        };
        println!(
            "{label:<28} {status:<10} {:>9.2?}  {}{}",
            t.elapsed(),
            stdout.trim(),
            stderr
                .lines()
                .find(|l| l.contains("panicked") || l.contains("overflow"))
                .unwrap_or("")
        );
    }
}

/// Runs one hostile input on the main thread (like the app would), in a
/// child process so a stack overflow does not hide the other results.
fn error_one(i: usize) {
    let (_, src) = &hostile()[i];
    match math::display_list(src, true, Color::BLACK) {
        Ok(dl) => {
            let m = math::metrics(&dl, 19.36, 4.0);
            let w = m.width.min(4000.0).ceil() as i32;
            let h = m.total_height.min(4000.0).ceil() as i32;
            let surf =
                cairo::ImageSurface::create(cairo::Format::ARgb32, w.max(1), h.max(1)).unwrap();
            let cr = cairo::Context::new(&surf).unwrap();
            let r = ratex_cairo::render_to_cairo(&cr, &dl, &math::options(19.36, 4.0));
            println!(
                "Ok: {} items, {:.0}x{:.0} px, render {:?}",
                dl.items.len(),
                m.width,
                m.total_height,
                r.err()
            );
        }
        Err(e) => println!("Err: {}", e.chars().take(90).collect::<String>()),
    }
}

/// Layout + render one formula to a small surface; returns the elapsed time.
fn layout_and_render(src: &str) -> (Duration, bool) {
    let t = Instant::now();
    let ok = match math::display_list(src, false, Color::BLACK) {
        Ok(dl) => {
            let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, 64, 64).unwrap();
            let cr = cairo::Context::new(&surf).unwrap();
            ratex_cairo::render_to_cairo(&cr, &dl, &math::options(19.36, 0.0)).is_ok()
        }
        Err(_) => false,
    };
    (t.elapsed(), ok)
}

/// Worst cases for one 1 MiB card side, everything on this (main) thread.
fn side() {
    const MIB: usize = 1 << 20;
    let cases: Vec<(&str, String)> = vec![
        (
            "2000-brace formulas",
            "{".repeat(2000) + "x" + &"}".repeat(2000),
        ),
        (
            "200-sqrt formulas",
            r"\sqrt{".repeat(200) + "x" + &"}".repeat(200),
        ),
        ("100k-char formula", "x".repeat(100_000)),
        ("$x$ (tiny valid)", "x".into()),
        ("#15 (largest valid)", formulas::DISPLAY[10].into()),
        (
            "x^{x^{x^{...}}} 20 deep",
            "x^{".repeat(20) + "x" + &"}".repeat(20),
        ),
    ];
    for (label, f) in cases {
        let n = MIB / (f.len() + 3);
        let one = layout_and_render(&f);
        let t = Instant::now();
        let mut ok = 0;
        for _ in 0..n {
            ok += usize::from(layout_and_render(&f).1);
        }
        println!(
            "{label:<26} one {:>9.2?}  ×{n:>6} in 1 MiB: {:>9.2?}  ({ok} rendered)",
            one.0,
            t.elapsed()
        );
    }
}

/// First use of a character outside the KaTeX fonts loads system fonts.
fn fallback() {
    for src in [r"\text{你好}", r"\text{😀}", r"\text{日本}", "x"] {
        let (t, ok) = layout_and_render(src);
        println!("{src:<16} {t:>10.2?} ok={ok}");
    }
}
