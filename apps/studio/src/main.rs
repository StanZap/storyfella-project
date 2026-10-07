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
        .with_cfg(dioxus::desktop::Config::new().with_window(window))
        .with_context(plan_launch)
        .launch(storyfella_studio::app::App);

    // The window closed and the event loop returned: stop the generation
    // server the app started rather than leaving it to outlive the process.
    storyfella_studio::shutdown::run();
}
