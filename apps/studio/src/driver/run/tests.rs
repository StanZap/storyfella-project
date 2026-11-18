use std::{cell::RefCell, path::Path, thread::JoinHandle};

use storyfella_studio_core::{config::AppConfig, errors::ReportedError, playback::PlaybackModel};

use super::*;
use crate::{commands::PendingOperations, host::Jobs};

/// The app as plain state on a thread of its own, serving a mailbox the way
/// the window does.
struct Fake {
    config: AppConfig,
    state: AppState,
    playback: PlaybackModel,
    guide: Option<GuideSession>,
    error: Option<ReportedError>,
    shown: Vec<Place>,
}

impl Driven for Fake {
    fn config(&self) -> &AppConfig {
        &self.config
    }
    fn state(&mut self) -> &mut AppState {
        &mut self.state
    }
    fn install_playback(
        &mut self,
        session: Result<storyfella_studio_core::playback::PlaybackSession, String>,
    ) {
        self.playback.install(session);
    }
    fn playback(&mut self, act: &mut dyn FnMut(&mut PlaybackModel)) {
        act(&mut self.playback);
    }
    fn guide(&mut self) -> &mut Option<GuideSession> {
        &mut self.guide
    }
    fn show(&mut self, place: Place) {
        self.shown.push(place);
    }
    fn set_palette(&mut self, _: bool) {}
    fn set_caret(&mut self, _: storyfella_language::Position) {}
    fn start(&mut self, operations: PendingOperations) {
        // Synchronous, so activity is idle again by the next call.
        self.error = commands::finish_operations(&mut self.state, Some(operations.run())).err();
    }
    fn activity(&self) -> OpActivity {
        OpActivity::Idle
    }
    fn error(&self) -> Option<ReportedError> {
        self.error.clone()
    }
    fn set_error(&mut self, error: Option<ReportedError>) {
        self.error = error;
    }
}

fn config(directory: &Path) -> AppConfig {
    let file = directory.join("studio.toml");
    std::fs::write(&file, "").unwrap();
    let mut config = AppConfig::load_from(&file).unwrap();
    config.project_dir = directory.join("projects");
    config.asset_dir = directory.join("artwork");
    config
}

fn serve(mut fake: Fake) -> (Mailbox, JoinHandle<Fake>) {
    let (mailbox, mut jobs): (Mailbox, Jobs) = Mailbox::new();
    let thread = std::thread::spawn(move || {
        // Jobs run outside the runtime: an operation starts its own.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        while let Some(job) = runtime.block_on(jobs.next()) {
            job(&mut fake);
        }
        fake
    });
    (mailbox, thread)
}

fn session(mode: GuideMode) -> GuideSession {
    GuideSession {
        plan: Plan::from_yaml("name: typing\nsteps: []\n").unwrap(),
        mode,
        current: 0,
        busy: None,
        paused: false,
        speed: 1,
        typing_cps: 200,
        state: GuideState::Running,
    }
}

fn fake(directory: &Path, mode: GuideMode) -> Fake {
    Fake {
        config: config(directory),
        state: AppState::default(),
        playback: PlaybackModel::default(),
        guide: Some(session(mode)),
        error: None,
        shown: Vec::new(),
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(future)
}

async fn set_guide(host: &Mailbox, guide: Option<GuideSession>) {
    host.call(move |app| *app.guide() = guide).await.unwrap();
}

#[test]
fn typing_obeys_pause_stop_and_mode_without_corrupting_unicode() {
    let directory = tempfile::tempdir().unwrap();
    let (host, thread) = serve(fake(directory.path(), GuideMode::Guide));
    block_on(async {
        let prefixes = RefCell::new(Vec::new());
        assert!(
            type_text(&host, "é🌊\r\n", async |text| prefixes
                .borrow_mut()
                .push(text.to_owned()))
            .await
        );
        assert_eq!(*prefixes.borrow(), ["é", "é🌊", "é🌊\r", "é🌊\r\n"]);

        // A paused Check run emits nothing until it is resumed.
        let mut paused = session(GuideMode::Check);
        paused.paused = true;
        set_guide(&host, Some(paused)).await;
        let count = RefCell::new(0);
        let typing = type_text(&host, "ab", async |_| *count.borrow_mut() += 1);
        let resume = async {
            tokio::time::sleep(Duration::from_millis(250)).await;
            assert_eq!(*count.borrow(), 0);
            host.call(|app| app.guide().as_mut().unwrap().paused = false)
                .await
                .unwrap();
        };
        let (typed, ()) = tokio::join!(typing, resume);
        assert!(typed);
        assert_eq!(*count.borrow(), 1);

        // Stop preserves the emitted prefix and never emits the rest.
        set_guide(&host, Some(session(GuideMode::Guide))).await;
        let partial = RefCell::new(String::new());
        assert!(
            !type_text(&host, "é🌊", async |text| {
                *partial.borrow_mut() = text.to_owned();
                set_guide(&host, None).await;
            })
            .await
        );
        assert_eq!(*partial.borrow(), "é");

        // Check mode emits the exact complete string once, without animation.
        set_guide(&host, Some(session(GuideMode::Check))).await;
        let emissions = RefCell::new(Vec::new());
        assert!(
            type_text(&host, "é🌊\r\n", async |text| emissions
                .borrow_mut()
                .push(text.to_owned()))
            .await
        );
        assert_eq!(*emissions.borrow(), ["é🌊\r\n"]);
    });
    drop(host);
    thread.join().unwrap();
}

#[test]
fn the_smoke_plan_passes_through_the_mailbox() {
    let directory = tempfile::tempdir().unwrap();
    let app = fake(directory.path(), GuideMode::Check);
    let config = app.config.clone();
    let (host, thread) = serve(app);
    let plan = Plan::from_yaml(include_str!("../../../../../config/plans/smoke.yaml")).unwrap();
    let report = block_on(run_steps(&host, &config, &plan, GuideMode::Check));
    assert_eq!(report.failed(), 0, "{:?}", report.steps);
    assert_eq!(report.steps.len(), plan.steps.len());
    drop(host);
    let app = thread.join().unwrap();
    assert_eq!(app.state.project_name, "Smoke Test");
    assert_eq!(app.shown, [Place::Write]);
}
