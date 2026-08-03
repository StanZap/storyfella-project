use tracing::Level;

fn main() {
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
    dioxus::launch(smart_visual_sequencer::app::App);
}
