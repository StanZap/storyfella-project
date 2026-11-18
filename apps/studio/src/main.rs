use tracing::Level;

fn main() {
    agentkit_ai::native::service::run_if_requested();
    let args: Vec<String> = std::env::args().collect();
    let launch = storyfella_studio::driver::PlanLaunch::from_args(&args);
    // A plan run is hermetic: it reads and writes scratch config under
    // `.svs-e2e/`, never the developer's real user config.
    launch.make_hermetic();

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
    storyfella_studio::ui::run(launch);
}
