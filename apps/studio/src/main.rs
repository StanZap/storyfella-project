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
    // the initial window unmaximized. Canvas rails and Preview need room.
    let window = dioxus::desktop::WindowBuilder::new()
        .with_title("Storyfella Studio")
        .with_inner_size(dioxus::desktop::LogicalSize::new(1280., 800.))
        .with_maximized(true);
    dioxus::LaunchBuilder::new()
        .with_cfg(
            dioxus::desktop::Config::new()
                .with_window(window)
                .with_menu(desktop_menu()),
        )
        .with_context(plan_launch)
        .launch(storyfella_studio::app::App);
}

fn desktop_menu() -> dioxus::desktop::muda::Menu {
    use dioxus::desktop::muda::{
        accelerator::{Accelerator, Code, Modifiers},
        Menu, MenuItem, PredefinedMenuItem, Submenu,
    };
    let menu = Menu::new();
    let app = Submenu::new("Storyfella", true);
    let modifier = if cfg!(target_os = "macos") {
        Modifiers::META
    } else {
        Modifiers::CONTROL
    };
    app.append_items(&[
        &PredefinedMenuItem::hide(None),
        &MenuItem::with_id(
            storyfella_studio::ui::QUIT_MENU_ID,
            "Quit Storyfella",
            true,
            Some(Accelerator::new(Some(modifier), Code::KeyQ)),
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
