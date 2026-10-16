//! Path 2: the inline widget from ../math on Cards' gtk-rs versions.
//!   cargo run --release [-- --dark] [-- --screenshot F.png]

#[path = "../../math/src/formulas.rs"]
mod formulas;
#[path = "../../math/src/inline.rs"]
mod inline;
#[path = "../../math/src/math.rs"]
mod math;

use std::time::Duration;

use adw::prelude::*;
use gtk::{gdk, glib, graphene};

pub const MATH_SCALE: f64 = 1.21;

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let app = adw::Application::builder()
        .application_id("io.github.rsmirke.Cards.MathSpike2")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    app.connect_activate(move |app| build(app, &args));
    app.run_with_args::<&str>(&[])
}

fn build(app: &adw::Application, args: &[String]) {
    let dark = args.iter().any(|a| a == "--dark");
    let shot = args
        .iter()
        .position(|a| a == "--screenshot")
        .map(|i| args[i + 1].clone());
    adw::StyleManager::default().set_color_scheme(if dark {
        adw::ColorScheme::ForceDark
    } else {
        adw::ColorScheme::ForceLight
    });
    let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_margin_top(12);
    content.set_margin_bottom(16);
    let desc = content.pango_context().font_description().unwrap();
    let size = f64::from(desc.size()) / f64::from(gtk::pango::SCALE);
    let em = if desc.is_size_absolute() {
        size
    } else {
        size * 96.0 / 72.0
    };
    let title = gtk::Label::builder()
        .label("Path 2: gtk4 0.11 + ratex-cairo on cairo 0.22")
        .xalign(0.0)
        .build();
    title.add_css_class("heading");
    content.append(&title);
    for s in formulas::INLINE {
        let pieces = formulas::split(s)
            .into_iter()
            .map(|(m, p)| (m, p.to_string()))
            .collect();
        content.append(&inline::InlineMath::new(pieces, em));
    }
    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());
    toolbar.set_content(Some(&content));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("RaTeX spike, path 2")
        .default_width(760)
        .content(&toolbar)
        .build();
    window.present();
    if let Some(path) = shot {
        let target: gtk::Widget = toolbar.upcast();
        let paintable = gtk::WidgetPaintable::new(Some(&target));
        glib::timeout_add_local_once(Duration::from_millis(600), move || {
            let (w, h) = (target.width() as f32, target.height() as f32);
            let snap = gtk::Snapshot::new();
            let bg = if adw::StyleManager::default().is_dark() {
                gdk::RGBA::new(0.133, 0.133, 0.149, 1.0)
            } else {
                gdk::RGBA::new(0.98, 0.98, 0.98, 1.0)
            };
            snap.append_color(&bg, &graphene::Rect::new(0.0, 0.0, w, h));
            paintable.snapshot(&snap, f64::from(w), f64::from(h));
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(None).unwrap();
            let tex = renderer.render_texture(
                snap.to_node().unwrap(),
                Some(&graphene::Rect::new(0.0, 0.0, w, h)),
            );
            renderer.unrealize();
            tex.save_to_png(&path).unwrap();
            eprintln!("wrote {path}");
            target.root().and_downcast::<gtk::Window>().unwrap().close();
        });
    }
}
