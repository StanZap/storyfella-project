//! What automation sees of the app, and how it reaches the app's thread.
//!
//! The plan driver and the live MCP bridge both run on their own tokio
//! threads: the driver sleeps and calls vision models, the bridge serves a
//! socket. The app's state lives on the UI thread. A [`Mailbox`] carries a
//! closure to that thread, runs it against [`Driven`], and returns its result,
//! so neither caller knows which toolkit draws the window.
use storyfella_language::Position;
use storyfella_studio_core::{
    config::AppConfig,
    errors::ReportedError,
    playback::{PlaybackModel, PlaybackSession},
};
use tokio::sync::{mpsc, oneshot};

use crate::{
    commands::{OpActivity, PendingOperations},
    driver::GuideSession,
    state::AppState,
};

/// Where Studio is showing its work.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Place {
    #[default]
    Home,
    Write,
    Play,
    Board,
    Settings,
}

/// The app as automation acts on it. Every method runs on the app's thread.
pub trait Driven {
    fn config(&self) -> &AppConfig;
    fn state(&mut self) -> &mut AppState;
    /// Installs a compiled preview, or reports why it did not compile.
    fn install_playback(&mut self, session: Result<PlaybackSession, String>);
    /// Runs `act` on the playback model.
    fn playback(&mut self, act: &mut dyn FnMut(&mut PlaybackModel));
    fn guide(&mut self) -> &mut Option<GuideSession>;
    fn show(&mut self, place: Place);
    fn set_palette(&mut self, open: bool);
    /// Moves the Write view's caret, scrolling it into view.
    fn set_caret(&mut self, position: Position);
    /// Runs operations begun with `commands::begin_operations` off the app's
    /// thread, then merges them with `commands::finish_operations`.
    fn start(&mut self, operations: PendingOperations);
    fn activity(&self) -> OpActivity;
    fn error(&self) -> Option<ReportedError>;
    fn set_error(&mut self, error: Option<ReportedError>);
}

type Job = Box<dyn FnOnce(&mut dyn Driven) + Send>;

/// A `Send` handle that runs closures on the app's thread.
#[derive(Clone)]
pub struct Mailbox(mpsc::UnboundedSender<Job>);

/// The app's end of a [`Mailbox`]: it runs each job against its [`Driven`].
pub struct Jobs(mpsc::UnboundedReceiver<Job>);

impl Mailbox {
    pub fn new() -> (Self, Jobs) {
        let (sender, receiver) = mpsc::unbounded_channel();
        (Self(sender), Jobs(receiver))
    }

    /// Runs `job` on the app's thread. `None` once the app has gone.
    pub async fn call<R: Send + 'static>(
        &self,
        job: impl FnOnce(&mut dyn Driven) -> R + Send + 'static,
    ) -> Option<R> {
        let (sender, receiver) = oneshot::channel();
        self.0
            .send(Box::new(move |app| {
                let _ = sender.send(job(app));
            }))
            .ok()?;
        receiver.await.ok()
    }
}

impl Jobs {
    /// The next job, or `None` once every mailbox has been dropped.
    pub async fn next(&mut self) -> Option<impl FnOnce(&mut dyn Driven)> {
        self.0.recv().await
    }
}

/// Runs `act` on the app's playback model and returns its result.
pub fn with_playback<R>(app: &mut dyn Driven, act: impl FnOnce(&mut PlaybackModel) -> R) -> R {
    let mut act = Some(act);
    let mut result = None;
    app.playback(&mut |model| result = act.take().map(|act| act(model)));
    result.expect("playback runs its closure once")
}
