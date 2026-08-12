use tracing::Level;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let plan_launch = smart_visual_sequencer::driver::PlanLaunch::from_args(&args);

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

    tracing::info!("starting Smart Visual Sequencer");
    // Maximized: the workspace is a canvas beside two rails, and at the
    // default window size the canvas is the part that loses.
    let window = dioxus::desktop::WindowBuilder::new()
        .with_title("Smart Visual Sequencer")
        .with_maximized(true);
    dioxus::LaunchBuilder::new()
        .with_cfg(dioxus::desktop::Config::new().with_window(window))
        .with_context(plan_launch)
        .launch(smart_visual_sequencer::app::App);
}
