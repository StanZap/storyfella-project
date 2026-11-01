//! Guarded review of two continuous documents. CodeMirror supplies diff ranges;
//! Rust validates ranges, source ownership, compiler correctness and application.
use super::writing::{WritingProposal, WritingRequest, WritingScope};
use crate::registry::ArtifactRegistry;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use storyfella_language::{Position, Source, Span};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRange {
    pub start: Position,
    pub end: Position,
}

impl ReviewRange {
    fn span(&self, source: &str) -> Result<Span> {
        let source = Source::new(source);
        let start = source
            .offset(self.start)
            .context("Invalid start of a review change")?;
        let end = source
            .offset(self.end)
            .context("Invalid end of a review change")?;
        ensure!(start <= end, "Invalid review change range");
        Ok(Span::new(start, end))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewChange {
    pub original: ReviewRange,
    pub proposed: ReviewRange,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritingReview {
    pub proposal: WritingProposal,
    pub draft: String,
    pub proposed: String,
    edit_base: String,
    manual_edits: Vec<ManualEdit>,
}

/// One editing period between model versions. Coalescing native keystrokes
/// avoids storing a document snapshot per keystroke, while retaining each
/// manual delta even when a later model happens to reproduce it verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ManualEdit {
    before: String,
    after: String,
}

impl WritingReview {
    pub fn new(mut proposal: WritingProposal, registry: &ArtifactRegistry) -> Result<Self> {
        proposal.replacement = proposal
            .request
            .normalize_replacement(&proposal.replacement);
        ensure!(
            !proposal.review(registry)?.has_errors,
            "The suggestion is no longer valid for the current story assets."
        );
        let draft = proposal.request.source.clone();
        let proposed = proposal.apply(
            &proposal.request.project,
            &proposal.request.source_path,
            &draft,
        )?;
        Ok(Self {
            proposal,
            draft,
            edit_base: proposed.clone(),
            proposed,
            manual_edits: Vec::new(),
        })
    }

    pub fn check_owner(&self, project: &Path, path: &Path, source: &str) -> Result<()> {
        ensure!(
            project == self.proposal.request.project && path == self.proposal.request.source_path,
            "Return to this suggestion's story to continue reviewing."
        );
        ensure!(source == self.draft,
            "The draft changed outside this review. Your suggestions are kept; return to the draft to start a fresh request.");
        Ok(())
    }

    pub fn edit(&mut self, text: &str) {
        self.proposed = storyfella_language::textarea_input(&self.proposed, text);
    }

    /// Construct a replacement review atomically. A conflict or an invalid
    /// combination leaves the previous review untouched; conflict markers
    /// are never inserted into story source or offered for acceptance.
    pub fn retry_review(
        &self,
        proposal: WritingProposal,
        registry: &ArtifactRegistry,
    ) -> Result<Self> {
        ensure!(
            proposal.request.project == self.proposal.request.project
                && proposal.request.source_path == self.proposal.request.source_path
                && proposal.request.source == self.draft,
            "This new suggestion belongs to a different draft."
        );
        let mut next = Self::new(proposal, registry)?;
        let mut edits = self.manual_edits.clone();
        if self.edit_base != self.proposed {
            edits.push(ManualEdit {
                before: self.edit_base.clone(),
                after: self.proposed.clone(),
            });
        }
        for edit in &edits {
            next.proposed = diffy::merge(&edit.before, &edit.after, &next.proposed)
                .map_err(|_| anyhow::anyhow!("The new version overlaps your manual edits. Your current review is kept. Revise the request, or finish this review before starting a new request."))?;
        }
        ensure!(!next.acceptance(None)?.review(registry)?.has_errors,
            "The new version could not be combined with your edits without source errors. Your current review is kept.");
        next.edit_base.clone_from(&next.proposed);
        next.manual_edits = edits;
        Ok(next)
    }

    /// A normal guarded proposal: AppState still owns validation, undo and saving.
    pub fn acceptance(&self, change: Option<&ReviewChange>) -> Result<WritingProposal> {
        let (range, replacement) = match change {
            Some(change) => {
                let original = change.original.span(&self.draft)?;
                let proposed = change.proposed.span(&self.proposed)?;
                (
                    original,
                    self.proposed[proposed.start..proposed.end].to_owned(),
                )
            }
            None => (Span::new(0, self.draft.len()), self.proposed.clone()),
        };
        let mut request = self.proposal.request.clone();
        request.source = self.draft.clone();
        request.range = range;
        Ok(WritingProposal {
            run_id: self.proposal.run_id.clone(),
            request,
            replacement,
        })
    }

    pub fn discard(&mut self, change: Option<&ReviewChange>) -> Result<()> {
        match change {
            Some(change) => {
                let original = change.original.span(&self.draft)?;
                let proposed = change.proposed.span(&self.proposed)?;
                self.proposed.replace_range(
                    proposed.start..proposed.end,
                    &self.draft[original.start..original.end],
                );
            }
            None => self.proposed = self.draft.clone(),
        }
        Ok(())
    }

    /// Resolve a group against one snapshot, so ranges cannot shift between
    /// edits and the host applies acceptance as one project undo step.
    pub fn acceptance_group(&self, changes: &[ReviewChange]) -> Result<WritingProposal> {
        let mut proposal = self.acceptance(None)?;
        proposal.replacement = self.resolve_group(changes, false)?;
        Ok(proposal)
    }

    pub fn discard_group(&mut self, changes: &[ReviewChange]) -> Result<()> {
        self.proposed = self.resolve_group(changes, true)?;
        Ok(())
    }

    fn resolve_group(&self, changes: &[ReviewChange], discard: bool) -> Result<String> {
        ensure!(!changes.is_empty(), "Select at least one change.");
        let (target, replacement) = if discard {
            (&self.proposed, &self.draft)
        } else {
            (&self.draft, &self.proposed)
        };
        let mut edits = changes
            .iter()
            .map(|change| {
                let (target_range, replacement_range) = if discard {
                    (&change.proposed, &change.original)
                } else {
                    (&change.original, &change.proposed)
                };
                let target = target_range.span(target)?;
                let replacement_range = replacement_range.span(replacement)?;
                Ok((
                    target,
                    replacement[replacement_range.start..replacement_range.end].to_owned(),
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        edits.sort_by_key(|(range, _)| (range.start, range.end));
        for pair in edits.windows(2) {
            ensure!(
                pair[0].0.end <= pair[1].0.start && pair[0].0 != pair[1].0,
                "Selected changes overlap."
            );
        }
        let mut result = target.clone();
        for (range, replacement) in edits.into_iter().rev() {
            result.replace_range(range.start..range.end, &replacement);
        }
        Ok(result)
    }

    pub fn finished(&self) -> bool {
        self.draft == self.proposed
    }

    pub fn retry_request(
        &self,
        changes: &[ReviewChange],
        registry: &ArtifactRegistry,
    ) -> Result<WritingRequest> {
        let range = changes
            .iter()
            .map(|change| change.original.span(&self.draft))
            .collect::<Result<Vec<_>>>()?;
        let range = Span::new(
            range.iter().map(|span| span.start).min().unwrap_or(0),
            range
                .iter()
                .map(|span| span.end)
                .max()
                .unwrap_or(self.draft.len()),
        );
        let scope = if range.start == range.end {
            WritingScope::Beat(range.start)
        } else {
            WritingScope::Selection(range)
        };
        let mut captured = WritingRequest::capture(
            &self.proposal.request.project,
            &self.proposal.request.source_path,
            &self.draft,
            scope,
            registry,
        )?;
        captured.action = self.proposal.request.action;
        captured.instructions = self.proposal.request.instructions.clone();
        captured.preserve_facts = self.proposal.request.preserve_facts;
        Ok(captured)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &str = "scene harbor\n  beat arrival\n    narrate: The sea is quiet.\n    # Keep this pause.\n    narrate: Mara waits.\n    end\n";
    fn review() -> WritingReview {
        let registry = ArtifactRegistry::new();
        let request = WritingRequest::capture(
            Path::new("story.db"),
            Path::new("story.sf"),
            SOURCE,
            WritingScope::Scene(0),
            &registry,
        )
        .unwrap();
        let replacement = request
            .original()
            .replace("quiet", "restless")
            .replace("waits", "speaks");
        WritingReview::new(
            WritingProposal {
                run_id: "review".into(),
                request,
                replacement,
            },
            &registry,
        )
        .unwrap()
    }
    fn lines(first: u32, last: u32) -> ReviewChange {
        let range = ReviewRange {
            start: Position {
                line: first,
                character: 0,
            },
            end: Position {
                line: last,
                character: 0,
            },
        };
        ReviewChange {
            original: range.clone(),
            proposed: range,
        }
    }

    fn next_proposal(review: &WritingReview, replacement: String) -> WritingProposal {
        let mut proposal = review.acceptance(None).unwrap();
        proposal.run_id = format!("{}-next", review.proposal.run_id);
        proposal.replacement = replacement;
        proposal
    }

    #[test]
    fn manual_edits_survive_retries_even_when_a_model_reproduces_them() {
        let registry = ArtifactRegistry::new();
        let mut current = review();
        current.edit(&current.proposed.replace("restless", "glittering 🌊"));
        let first = current
            .retry_review(
                next_proposal(
                    &current,
                    SOURCE
                        .replace("quiet", "restless")
                        .replace("waits", "sings"),
                ),
                &registry,
            )
            .unwrap();
        assert_eq!(
            first.proposed,
            SOURCE
                .replace("quiet", "glittering 🌊")
                .replace("waits", "sings")
        );
        assert_eq!(first.draft, SOURCE);
        let second = first
            .retry_review(
                next_proposal(
                    &first,
                    SOURCE
                        .replace("quiet", "glittering 🌊")
                        .replace("waits", "hums"),
                ),
                &registry,
            )
            .unwrap();
        // Reopening and retrying again must not forget that this text came
        // from the user merely because the preceding model reproduced it.
        let mut reopened: WritingReview =
            serde_json::from_str(&serde_json::to_string(&second).unwrap()).unwrap();
        let third = reopened
            .retry_review(
                next_proposal(
                    &reopened,
                    SOURCE
                        .replace("quiet", "restless")
                        .replace("waits", "listens"),
                ),
                &registry,
            )
            .unwrap();
        assert!(third.proposed.contains("glittering 🌊"));
        reopened = third;
        reopened.edit(&reopened.proposed.replace("glittering 🌊", "silver"));
        let fourth = reopened
            .retry_review(
                next_proposal(
                    &reopened,
                    SOURCE
                        .replace("quiet", "restless")
                        .replace("waits", "shouts"),
                ),
                &registry,
            )
            .unwrap();
        assert_eq!(
            fourth.proposed,
            SOURCE.replace("quiet", "silver").replace("waits", "shouts")
        );
    }

    #[test]
    fn overlapping_rewrites_leave_the_current_manual_review_untouched() {
        let mut current = review();
        current.edit(&current.proposed.replace("restless", "glittering"));
        let before = current.clone();
        let result = current.retry_review(
            next_proposal(
                &current,
                SOURCE.replace("quiet", "rough").replace("waits", "sings"),
            ),
            &ArtifactRegistry::new(),
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("overlaps your manual edits"));
        assert_eq!(current, before);
        assert!(!current.proposed.contains("<<<<<<<"));
    }

    #[test]
    fn added_and_deleted_user_lines_survive_unrelated_rewrites() {
        let registry = ArtifactRegistry::new();
        for (original, replacement) in [
            (
                "    # Keep this pause.\n",
                "    narrate: My added line.\n    # Keep this pause.\n",
            ),
            ("    # Keep this pause.\n", ""),
        ] {
            let mut current = review();
            current.edit(&current.proposed.replace(original, replacement));
            let next = current
                .retry_review(
                    next_proposal(
                        &current,
                        current
                            .edit_base
                            .replace("scene harbor", "scene harbor_dawn"),
                    ),
                    &registry,
                )
                .unwrap();
            assert_eq!(
                next.proposed,
                current
                    .proposed
                    .replace("scene harbor", "scene harbor_dawn")
            );
        }
    }

    #[test]
    fn adjacent_deletion_and_rewrite_do_not_drop_the_deleted_line_choice() {
        let registry = ArtifactRegistry::new();
        let mut current = review();
        current.edit(&current.proposed.replace("    # Keep this pause.\n", ""));
        let before = current.clone();
        let result = current.retry_review(
            next_proposal(&current, current.edit_base.replace("speaks", "sings")),
            &registry,
        );
        match result {
            Ok(next) => {
                assert!(!next.proposed.contains("Keep this pause"));
                assert!(next.proposed.contains("Mara sings"));
            }
            Err(_) => assert_eq!(current, before),
        }
    }

    #[test]
    fn manual_retry_merge_preserves_crlf_and_unicode() {
        let registry = ArtifactRegistry::new();
        let source = SOURCE.replace('\n', "\r\n");
        let mut proposal = review().proposal;
        proposal.request.source.clone_from(&source);
        proposal.request.range = Span::new(0, source.len());
        proposal.replacement = source
            .replace("quiet", "restless")
            .replace("waits", "speaks");
        let mut current = WritingReview::new(proposal, &registry).unwrap();
        current.edit(
            &current
                .proposed
                .replace("\r\n", "\n")
                .replace("restless", "cálm 🌊"),
        );
        let next = current
            .retry_review(
                next_proposal(&current, current.edit_base.replace("speaks", "sings")),
                &registry,
            )
            .unwrap();
        assert_eq!(
            next.proposed,
            source.replace("quiet", "cálm 🌊").replace("waits", "sings")
        );
    }

    #[test]
    fn a_clean_text_merge_that_breaks_story_links_is_not_published() {
        let mut current = review();
        current.edit(&current.proposed.replace("scene harbor", "scene lagoon"));
        let before = current.clone();
        let candidate = before.edit_base.replace("    end\n", "    goto harbor\n");
        let result =
            current.retry_review(next_proposal(&current, candidate), &ArtifactRegistry::new());
        assert!(result.unwrap_err().to_string().contains("source errors"));
        assert_eq!(current, before);
    }

    #[test]
    fn reset_choices_and_accepted_manual_text_survive_a_retry() {
        let registry = ArtifactRegistry::new();
        let mut current = review();
        current.discard(Some(&lines(2, 3))).unwrap();
        let reset = current
            .retry_review(
                next_proposal(&current, current.edit_base.clone()),
                &registry,
            )
            .unwrap();
        assert!(reset.proposed.contains("sea is quiet"));
        assert!(reset.proposed.contains("Mara speaks"));

        let mut current = review();
        current.edit(&current.proposed.replace("restless", "glittering"));
        current.draft = current
            .acceptance(Some(&lines(2, 3)))
            .unwrap()
            .apply(Path::new("story.db"), Path::new("story.sf"), SOURCE)
            .unwrap();
        let next = current
            .retry_review(
                next_proposal(&current, current.draft.replace("waits", "sings")),
                &registry,
            )
            .unwrap();
        assert!(next.proposed.contains("glittering"));
        assert!(next.proposed.contains("Mara sings"));
        assert_eq!(next.draft, current.draft);
    }
    #[test]
    fn partial_acceptance_and_discard_preserve_other_changes_and_manual_edits() {
        let mut review = review();
        review.edit(&review.proposed.replace("restless", "glittering"));
        assert_eq!(review.draft, SOURCE);
        let proposal = review.acceptance(Some(&lines(2, 3))).unwrap();
        review.draft = proposal
            .apply(Path::new("story.db"), Path::new("story.sf"), SOURCE)
            .unwrap();
        assert_eq!(review.draft, SOURCE.replace("quiet", "glittering"));
        review.discard(Some(&lines(4, 5))).unwrap();
        assert!(review.finished());
        assert!(review.proposed.contains("glittering"));
        assert!(review.proposed.contains("Mara waits."));
    }
    #[test]
    fn group_actions_resolve_only_checked_changes_against_one_snapshot() {
        let mut review = review();
        review.proposed = review.proposed.replace("Keep this pause", "A longer pause");
        let selected = [lines(4, 5), lines(2, 3)];
        let proposal = review.acceptance_group(&selected).unwrap();
        assert_eq!(
            proposal.replacement,
            SOURCE
                .replace("quiet", "restless")
                .replace("waits", "speaks")
        );
        assert!(
            !proposal
                .review(&ArtifactRegistry::new())
                .unwrap()
                .has_errors
        );
        review.discard_group(&selected).unwrap();
        assert_eq!(
            review.proposed,
            SOURCE.replace("Keep this pause", "A longer pause")
        );
        let before = review.clone();
        assert!(review.discard_group(&[lines(2, 4), lines(3, 5)]).is_err());
        assert!(review.discard_group(&[lines(2, 3), lines(2, 3)]).is_err());
        assert!(review.acceptance_group(&[]).is_err());
        assert_eq!(review, before);
    }

    #[test]
    fn group_insertions_and_deletions_use_their_own_document_coordinates() {
        let mut review = review();
        review.proposed = SOURCE.replace("    # Keep this pause.\n", "").replace(
            "    end\n",
            "    narrate: One.\n    narrate: Two.\n    end\n",
        );
        let selected = [
            ReviewChange {
                original: lines(3, 4).original,
                proposed: lines(3, 3).proposed,
            },
            ReviewChange {
                original: lines(5, 5).original,
                proposed: lines(4, 6).proposed,
            },
        ];
        assert_eq!(
            review.acceptance_group(&selected).unwrap().replacement,
            review.proposed
        );
        review.discard_group(&selected).unwrap();
        assert_eq!(review.proposed, SOURCE);
    }
    #[test]
    fn insertion_and_deletion_are_guarded_edits_too() {
        let mut review = review();
        review.proposed = SOURCE.replace("    # Keep this pause.\n", "");
        let deletion = ReviewChange {
            original: lines(3, 4).original,
            proposed: lines(3, 3).proposed,
        };
        let proposal = review.acceptance(Some(&deletion)).unwrap();
        assert_eq!(
            proposal
                .apply(Path::new("story.db"), Path::new("story.sf"), SOURCE)
                .unwrap(),
            review.proposed
        );
        review.discard(Some(&deletion)).unwrap();
        assert!(review.finished());
        review.proposed = format!("{SOURCE}# A new ending\n");
        let insertion = ReviewChange {
            original: lines(6, 6).original,
            proposed: lines(6, 7).proposed,
        };
        let proposal = review.acceptance(Some(&insertion)).unwrap();
        assert_eq!(
            proposal
                .apply(Path::new("story.db"), Path::new("story.sf"), SOURCE)
                .unwrap(),
            review.proposed
        );
    }
    #[test]
    fn invalid_ranges_and_wrong_owners_are_rejected_without_mutation() {
        let mut review = review();
        let before = review.clone();
        assert!(review.discard(Some(&lines(300, 400))).is_err());
        assert_eq!(review, before);
        assert!(review
            .check_owner(Path::new("other.db"), Path::new("story.sf"), SOURCE)
            .is_err());
        assert!(review
            .check_owner(Path::new("story.db"), Path::new("story.sf"), "changed")
            .is_err());
    }
    #[test]
    fn crlf_unicode_and_retry_keep_the_accepted_snapshot() {
        let mut review = review();
        review.draft = SOURCE.replace('\n', "\r\n");
        review.proposed = review.draft.replace("quiet", "é 😀");
        review.edit(
            &review
                .proposed
                .replace("\r\n", "\n")
                .replace("waits", "listens"),
        );
        assert!(review.proposed.contains("é 😀.\r\n"));
        let request = review
            .retry_request(&[lines(2, 3), lines(4, 5)], &ArtifactRegistry::new())
            .unwrap();
        assert_eq!(request.source, review.draft);
        assert_eq!(
            request.original(),
            &review.draft[Source::new(&review.draft)
                .offset(Position {
                    line: 2,
                    character: 0
                })
                .unwrap()
                ..Source::new(&review.draft)
                    .offset(Position {
                        line: 5,
                        character: 0
                    })
                    .unwrap()]
        );
    }
}
