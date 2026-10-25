//! Session activity: failures belong to an operation, not the current page.
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    App,
    Library,
    Project(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Task {
    Generation(u64),
    Save(PathBuf),
    Recovery,
    Import,
    Export,
    Library,
    Mask(String),
    Settings,
    Open,
    Other,
}

impl Task {
    pub fn title(&self) -> &'static str {
        match self {
            Self::Generation(_) => "Generation failed",
            Self::Save(_) => "Project not saved",
            Self::Recovery => "Recovery not saved",
            Self::Import => "Import failed",
            Self::Export => "Export failed",
            Self::Library => "Library needs attention",
            Self::Mask(_) => "Mask not saved",
            Self::Settings => "Settings not saved",
            Self::Open => "Could not open project",
            Self::Other => "Action needs attention",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    RateLimit,
    Credentials,
    Connection,
    Other,
}

impl Cause {
    // Adapters return anyhow chains. Only recognize unambiguous HTTP/network
    // wording here; provider wire formats and retry timing stay in Clipforge.
    pub fn from_details(details: &str) -> Self {
        let text = details.to_lowercase();
        if text.contains("429 too many requests")
            || text.contains("rate limit")
            || text.contains("request was throttled")
        {
            Self::RateLimit
        } else if text.contains("401 unauthorized")
            || text.contains("403 forbidden")
            || text.contains("invalid api key")
        {
            Self::Credentials
        } else if text.contains("connection refused")
            || text.contains("connection reset")
            || text.contains("timed out")
            || text.contains("dns error")
        {
            Self::Connection
        } else {
            Self::Other
        }
    }
}

pub struct Report {
    pub id: u64,
    pub scope: Scope,
    pub task: Task,
    pub context: Option<String>,
    pub project_name: Option<String>,
    pub cause: Cause,
    pub attempts: usize,
    pub diagnostics: Vec<String>,
    pub dismissed: bool,
    pub resolved: bool,
    /// The newest failed attempt, whose captured inputs Retry must use.
    pub retry_request: Option<u64>,
}

impl Report {
    pub fn needs_attention(&self) -> bool {
        !self.dismissed && !self.resolved
    }

    pub fn explanation(&self) -> &'static str {
        if matches!(self.task, Task::Generation(_)) {
            return match self.cause {
                Cause::RateLimit => "The provider is limiting requests. Try again shortly.",
                Cause::Credentials => "Check this provider in Clipforge, then try again.",
                Cause::Connection => {
                    "Could not reach the provider. Check the connection and try again."
                }
                Cause::Other => {
                    "The request did not finish. Review the details before trying again."
                }
            };
        }
        match self.task {
            Task::Save(_) | Task::Recovery => {
                "Check the save destination and try again. Your changes remain open."
            }
            Task::Import => "The images could not be imported. Check the files and try again.",
            Task::Export => "The export did not finish. Check the destination and try again.",
            Task::Library => {
                "The shared library could not be updated or loaded. Try refreshing it."
            }
            Task::Mask(_) => {
                "The mask is still open. Try saving it again before leaving this image."
            }
            Task::Settings => "The settings could not be saved. Review them and try again.",
            Task::Open => "The project could not be opened. Check the file and try again.",
            Task::Other | Task::Generation(_) => "Review the details before trying again.",
        }
    }

    pub fn text(&self) -> String {
        let mut text = self.task.title().to_string();
        if let Some(name) = &self.project_name {
            text.push_str(&format!("\nProject: {name}"));
        }
        for (index, detail) in self.diagnostics.iter().enumerate() {
            text.push_str(&format!("\n\nReport {}\n{detail}", index + 1));
        }
        text
    }
}

#[derive(Default)]
pub struct Activity {
    reports: Vec<Report>,
    next_id: u64,
}

impl Activity {
    pub fn reports(&self) -> &[Report] {
        &self.reports
    }
    pub fn get(&self, id: u64) -> Option<&Report> {
        self.reports.iter().find(|report| report.id == id)
    }
    pub fn get_mut(&mut self, id: u64) -> Option<&mut Report> {
        self.reports.iter_mut().find(|report| report.id == id)
    }
    pub fn count(&self) -> usize {
        self.reports
            .iter()
            .filter(|report| report.needs_attention())
            .count()
    }

    pub fn record(
        &mut self,
        scope: Scope,
        task: Task,
        context: Option<String>,
        project_name: Option<String>,
        details: String,
    ) -> u64 {
        // Exact repeats share a report; a different failure never replaces the
        // report the person may currently be reading.
        if let Some(report) = self.reports.iter_mut().rev().find(|report| {
            !report.resolved
                && report.scope == scope
                && report.task == task
                && report.context == context
                && report.diagnostics.last() == Some(&details)
        }) {
            report.attempts += 1;
            report.dismissed = false;
            return report.id;
        }
        self.next_id += 1;
        let retry_request = if let Task::Generation(id) = task {
            Some(id)
        } else {
            None
        };
        let id = self.next_id;
        self.reports.push(Report {
            id,
            scope,
            task,
            context,
            project_name,
            cause: Cause::from_details(&details),
            attempts: 1,
            diagnostics: vec![details],
            dismissed: false,
            resolved: false,
            retry_request,
        });
        id
    }

    pub fn repeat_generation(&mut self, id: u64, request: u64, details: String) {
        if let Some(report) = self.get_mut(id) {
            report.cause = Cause::from_details(&details);
            report.attempts += 1;
            if report.diagnostics.last() != Some(&details) {
                report.diagnostics.push(details);
            }
            report.retry_request = Some(request);
            report.dismissed = false;
        }
    }

    pub fn resolve(&mut self, scope: &Scope, task: &Task) {
        for report in &mut self.reports {
            if &report.scope == scope && &report.task == task {
                report.resolved = true;
            }
        }
    }

    pub fn latest(&self, scope: &Scope, context: Option<&str>) -> Option<&Report> {
        self.reports.iter().rev().find(|report| {
            report.needs_attention()
                && &report.scope == scope
                && report.context.as_deref() == context
        })
    }

    pub fn save_failure(&self, epoch: u64) -> Option<&Report> {
        self.reports.iter().rev().find(|report| {
            !report.resolved
                && report.scope == Scope::Project(epoch)
                && matches!(report.task, Task::Save(_))
        })
    }
}

#[derive(Clone)]
pub struct PromptValidation {
    pub epoch: u64,
    pub target: String,
    pub message: String,
}

impl Activity {
    pub fn for_task(&self, scope: &Scope, task: &Task) -> Option<&Report> {
        self.reports
            .iter()
            .rev()
            .find(|report| !report.resolved && &report.scope == scope && &report.task == task)
    }

    pub fn clear_completed(&mut self, selected: Option<u64>) {
        self.reports.retain(|report| {
            Some(report.id) == selected
                || !report.resolved
                    && (!report.dismissed
                        || matches!(report.task, Task::Save(_) | Task::Recovery | Task::Mask(_)))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_keep_their_project_and_composer_context_and_do_not_replace_other_reports() {
        let mut activity = Activity::default();
        let first = activity.record(
            Scope::Project(3),
            Task::Generation(1),
            Some("canvas:1:full".into()),
            Some("Character".into()),
            "HTTP 429 Too Many Requests: low provider credit".into(),
        );
        let saved_text = activity.get(first).unwrap().text();
        let second = activity.record(
            Scope::Project(7),
            Task::Save("outfit.atelier".into()),
            None,
            None,
            "Permission denied".into(),
        );
        assert_ne!(first, second);
        assert!(activity
            .latest(&Scope::Project(7), Some("canvas:1:full"))
            .is_none());
        assert!(activity.latest(&Scope::Project(3), Some("new")).is_none());
        assert_eq!(
            activity
                .latest(&Scope::Project(3), Some("canvas:1:full"))
                .unwrap()
                .id,
            first
        );
        let report = activity.get(first).unwrap();
        assert_eq!(report.cause, Cause::RateLimit);
        assert!(!report.explanation().contains("credit"));
        assert!(report.text().contains("low provider credit"));
        assert_eq!(report.text(), saved_text);
    }

    #[test]
    fn repeated_attempts_share_attention_and_preserve_diagnostics() {
        let mut activity = Activity::default();
        let id = activity.record(
            Scope::Project(3),
            Task::Generation(1),
            Some("new".into()),
            None,
            "HTTP 429 Too Many Requests".into(),
        );
        assert_eq!(
            id,
            activity.record(
                Scope::Project(3),
                Task::Generation(1),
                Some("new".into()),
                None,
                "HTTP 429 Too Many Requests".into()
            )
        );
        activity.repeat_generation(id, 2, "Connection refused".into());
        assert_eq!(activity.count(), 1);
        let report = activity.get(id).unwrap();
        assert_eq!(report.attempts, 3);
        assert_eq!(report.retry_request, Some(2));
        assert_eq!(report.diagnostics.len(), 2);
        assert!(report.text().contains("429 Too Many Requests"));
        assert_eq!(report.cause, Cause::Connection);
        activity.get_mut(id).unwrap().resolved = true;
        assert_eq!(activity.count(), 0);
    }

    #[test]
    fn dismissing_and_clearing_attention_does_not_claim_unsaved_work_is_saved() {
        let mut activity = Activity::default();
        let task = Task::Save("character.atelier".into());
        let id = activity.record(
            Scope::Project(3),
            task.clone(),
            None,
            None,
            "No space left".into(),
        );
        activity.get_mut(id).unwrap().dismissed = true;
        assert_eq!(activity.count(), 0);
        activity.clear_completed(None);
        assert_eq!(activity.save_failure(3).unwrap().id, id);
        activity.resolve(&Scope::Project(7), &task);
        assert!(activity.save_failure(3).is_some());
        activity.resolve(&Scope::Project(3), &Task::Import);
        assert!(activity.save_failure(3).is_some());
        activity.resolve(&Scope::Project(3), &task);
        assert!(activity.save_failure(3).is_none());
        activity.clear_completed(Some(id));
        assert!(activity.get(id).is_some());
        activity.clear_completed(None);
        assert!(activity.get(id).is_none());
    }
}
