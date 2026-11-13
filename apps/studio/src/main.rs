use tracing::Level;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let plan_launch = storyfella_studio::driver::PlanLaunch::from_args(&args);
    // A plan run is hermetic: it reads and writes scratch config under
    // `.svs-e2e/`, never the developer's real user config.
    plan_launch.make_hermetic();

    let level = if cfg!(debug_assertions) {
        Level::DEBUG
    } else {
        Level::INFO
    };

    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_target(true)
        .compact()
        .init();

    tracing::info!("starting Storyfella Studio");
    // Maximize the workspace, with a wide fallback for platforms that leave
    // the initial window unmaximized. Assets rails and Preview need room.
    let (width, height) = plan_launch.window_size.unwrap_or((1280, 800));
    let window = dioxus::desktop::WindowBuilder::new()
        .with_title("Storyfella Studio")
        .with_inner_size(dioxus::desktop::LogicalSize::new(
            f64::from(width),
            f64::from(height),
        ))
        .with_maximized(plan_launch.window_size.is_none());
    dioxus::LaunchBuilder::new()
        .with_cfg(launch_config(window))
        .with_context(plan_launch)
        .launch(storyfella_studio::app::App);
}

/// The desktop configuration for this platform.
///
/// macOS needs the native menu bar: its Edit submenu supplies cut/copy/paste
/// and select-all to the webview. On Linux and Windows Dioxus draws that bar
/// inside the window, where the workspace's own controls cover those actions,
/// so neither the custom menu nor the default one is wanted there.
fn launch_config(window: dioxus::desktop::WindowBuilder) -> dioxus::desktop::Config {
    let config = dioxus::desktop::Config::new().with_window(window);
    #[cfg(target_os = "macos")]
    let config = config.with_menu(desktop_menu());
    #[cfg(not(target_os = "macos"))]
    let config = config.with_menu(None::<dioxus::desktop::muda::Menu>);
    config
}

/// The macOS menu bar; [`launch_config`] explains why it is macOS-only.
#[cfg(target_os = "macos")]
fn desktop_menu() -> dioxus::desktop::muda::Menu {
    use dioxus::desktop::muda::{
        accelerator::{Accelerator, Code, Modifiers},
        Menu, MenuItem, PredefinedMenuItem, Submenu,
    };
    let menu = Menu::new();
    let app = Submenu::new("Storyfella", true);
    app.append_items(&[
        &PredefinedMenuItem::hide(None),
        &MenuItem::with_id(
            storyfella_studio::ui::QUIT_MENU_ID,
            "Quit Storyfella",
            true,
            Some(Accelerator::new(Some(Modifiers::META), Code::KeyQ)),
        ),
    ])
    .expect("application menu");
    let edit = Submenu::new("Edit", true);
    edit.append_items(&[
        &PredefinedMenuItem::undo(None),
        &PredefinedMenuItem::redo(None),
        &PredefinedMenuItem::cut(None),
        &PredefinedMenuItem::copy(None),
        &PredefinedMenuItem::paste(None),
        &PredefinedMenuItem::select_all(None),
    ])
    .expect("edit menu");
    menu.append_items(&[&app, &edit]).expect("menu bar");
    menu
}
