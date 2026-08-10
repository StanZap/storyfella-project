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
    dioxus::LaunchBuilder::new()
        .with_context(plan_launch)
        .launch(smart_visual_sequencer::app::App);
}
