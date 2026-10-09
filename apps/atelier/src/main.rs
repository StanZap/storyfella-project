mod app;
mod canvas_renderer;
mod document;
#[cfg(target_os = "macos")]
mod gpu_canvas;
#[cfg(target_os = "macos")]
mod gpu_surface;
mod hosted_images;
mod providers;
mod stroke_curve;
mod tablet;
#[cfg(test)]
mod tests;
mod ui;
mod viewport;
mod worker;
use app::DrawingApp;
use gpui_kit::*;
use std::path::PathBuf;
gpui_kit::actions!(atelier, [Quit]);
fn main() {
    let mut args = std::env::args().skip(1);
    let data_dir = match args.next().as_deref() {
        Some("--data-dir") => PathBuf::from(args.next().expect("--data-dir requires a directory")),
        Some("--help") => {
            println!("Atelier · cargo run -p storyfella-atelier -- [--data-dir DIRECTORY]");
            return;
        }
        Some(arg) => panic!("Unknown argument: {arg}"),
        None => {
            let base = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            #[cfg(target_os = "macos")]
            let base = base.join("Library/Application Support");
            #[cfg(not(target_os = "macos"))]
            let base = base.join(".local/share");
            base.join("Atelier")
        }
    };
    if let Err(error) = std::fs::create_dir_all(&data_dir) {
        eprintln!("Could not create Atelier data directory: {error}");
        return;
    }
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.on_action(|_: &Quit, cx| cx.quit());
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            cx.set_menus([Menu::new("Atelier").items([MenuItem::action("Quit Atelier", Quit)])]);
            gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Light, None, cx);
            gpui_kit::component::Theme::update(cx, |theme| {
                theme.colors.primary = rgb(0x790e26).into();
                theme.colors.primary_hover = rgb(0x650d20).into();
                theme.colors.primary_active = rgb(0x520b1b).into();
                theme.colors.button_primary = rgb(0x790e26).into();
                theme.colors.button_primary_hover = rgb(0x650d20).into();
                theme.colors.button_primary_active = rgb(0x520b1b).into();
                theme.colors.button_primary_foreground = rgb(0xffffff).into();
                theme.colors.accent = rgb(0xf1e6eb).into();
                theme.colors.accent_foreground = rgb(0x790e26).into();
                theme.colors.caret = rgb(0x790e26).into();
            });
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Atelier · Drawing studies".into()),
                        ..Default::default()
                    }),
                    window_bounds: Some(WindowBounds::centered(size(px(1360.), px(900.)), cx)),
                    window_min_size: Some(size(px(1000.), px(700.))),
                    ..Default::default()
                },
                cx,
                |window, cx| cx.new(|cx| DrawingApp::new(data_dir, window, cx)),
            )
            .expect("Open Atelier window");
            cx.activate(true);
        });
}
