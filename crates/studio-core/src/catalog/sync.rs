//! Three-way catalog refresh. UUIDs identify records; independent fields merge,
//! while overlapping edits require an explicit editor decision.
use std::collections::HashSet;

use uuid::Uuid;

use crate::registry::{Artifact, ArtifactRegistry};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Resolution {
    #[default]
    Check,
    Local,
    Saved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    pub fields: Vec<ConflictingField>,
    /// A name/relationship collision cannot be resolved by choosing fields.
    pub requires_reload: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictingField {
    pub label: String,
    pub local: String,
    pub saved: String,
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Catalog changes overlap: {}",
            self.fields
                .iter()
                .map(|field| field.label.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

impl std::error::Error for Conflict {}

struct Merge {
    resolution: Resolution,
    conflicts: Vec<ConflictingField>,
}

impl Merge {
    fn field<T: Clone + PartialEq + std::fmt::Debug>(
        &mut self,
        label: &str,
        base: &T,
        local: &T,
        saved: &T,
    ) -> T {
        if local == base || local == saved {
            return saved.clone();
        }
        if saved == base {
            return local.clone();
        }
        self.conflicts.push(ConflictingField {
            label: label.to_owned(),
            local: format!("{local:?}"),
            saved: format!("{saved:?}"),
        });
        match self.resolution {
            Resolution::Saved => saved.clone(),
            Resolution::Local | Resolution::Check => local.clone(),
        }
    }

    fn records<T: Clone + PartialEq + std::fmt::Debug>(
        &mut self,
        label: &str,
        base: &[T],
        local: &[T],
        saved: &[T],
        id: impl Fn(&T) -> Uuid,
        merge: impl Fn(&mut Self, &T, &T, &T) -> T,
    ) -> Vec<T> {
        let mut seen = HashSet::new();
        local
            .iter()
            .chain(saved)
            .chain(base)
            .filter_map(|item| {
                let key = id(item);
                if !seen.insert(key) {
                    return None;
                }
                let b = base.iter().find(|item| id(item) == key);
                let l = local.iter().find(|item| id(item) == key);
                let s = saved.iter().find(|item| id(item) == key);
                if let (Some(b), Some(l), Some(s)) = (b, l, s) {
                    Some(merge(self, b, l, s))
                } else {
                    self.field(&format!("{label} {key}"), &b, &l, &s).cloned()
                }
            })
            .collect()
    }

    fn artifact(&mut self, b: &Artifact, l: &Artifact, s: &Artifact) -> Artifact {
        let label = &l.name;
        macro_rules! field {
            ($name:ident, $title:literal) => {
                self.field(
                    &format!("{label}: {}", $title),
                    &b.$name,
                    &l.$name,
                    &s.$name,
                )
            };
        }
        Artifact {
            id: l.id,
            kind: field!(kind, "Kind"),
            name: field!(name, "Name"),
            description: field!(description, "Description"),
            variant_of: field!(variant_of, "Variation base"),
            variant_axis: field!(variant_axis, "Variation type"),
            default_size: field!(default_size, "Image size"),
            default_steps: field!(default_steps, "Sampling steps"),
            default_seed: field!(default_seed, "Seed"),
            parent_id: field!(parent_id, "Parent"),
            composition: field!(composition, "Composition"),
            active_revision_id: field!(active_revision_id, "Selected artwork"),
            revisions: self.records(
                &format!("{label}: artwork"),
                &b.revisions,
                &l.revisions,
                &s.revisions,
                |r| r.id,
                |m, b, l, s| m.field(&format!("{label}: artwork {}", l.id), b, l, s),
            ),
            drafts: self.records(
                &format!("{label}: proposal"),
                &b.drafts,
                &l.drafts,
                &s.drafts,
                |d| d.id,
                |m, b, l, s| m.field(&format!("{label}: proposal {}", l.id), b, l, s),
            ),
        }
    }
}

pub fn merge(
    base: &ArtifactRegistry,
    local: &ArtifactRegistry,
    saved: &ArtifactRegistry,
    resolution: Resolution,
) -> Result<ArtifactRegistry, Conflict> {
    let mut m = Merge {
        resolution,
        conflicts: Vec::new(),
    };
    let result = ArtifactRegistry {
        story_scenes: m.field(
            "Story structure",
            &base.story_scenes,
            &local.story_scenes,
            &saved.story_scenes,
        ),
        artifacts: m.records(
            "Item",
            &base.artifacts,
            &local.artifacts,
            &saved.artifacts,
            |a| a.id,
            Merge::artifact,
        ),
        loras: m.records(
            "Adapter",
            &base.loras,
            &local.loras,
            &saved.loras,
            |a| a.id,
            |m, b, l, s| m.field(&format!("Adapter {}", l.name), b, l, s),
        ),
        log: m.records(
            "History",
            &base.log,
            &local.log,
            &saved.log,
            |e| e.id,
            |m, b, l, s| m.field(&format!("History {}", l.id), b, l, s),
        ),
    };
    let mut invalid = Vec::new();
    for (index, artifact) in result.artifacts.iter().enumerate() {
        // Existing registries may contain ambiguous names (resolution already
        // reports those). Refuse a collision introduced by combining editors,
        // without making an otherwise unchanged saved project unsaveable.
        let already_together = |registry: &ArtifactRegistry, other_id| {
            registry
                .artifact(artifact.id)
                .zip(registry.artifact(other_id))
                .is_some_and(|(a, b)| a.name.eq_ignore_ascii_case(&b.name))
        };
        if super::KINDS.contains(&artifact.kind)
            && result.artifacts[..index].iter().any(|other| {
                other.name.eq_ignore_ascii_case(&artifact.name)
                    && !already_together(local, other.id)
                    && !already_together(saved, other.id)
            })
        {
            invalid.push(format!("Two items use the name {}", artifact.name));
        }
        if artifact
            .parent_id
            .into_iter()
            .chain(artifact.variant_of)
            .any(|id| result.artifact(id).is_none())
            || artifact
                .active_revision_id
                .is_some_and(|id| !artifact.revisions.iter().any(|r| r.id == id))
            || artifact.composition.as_ref().is_some_and(|c| {
                c.layers.iter().any(|layer| {
                    result.artifact(layer.artifact_ref).is_none()
                        || layer
                            .variant_ref
                            .is_some_and(|id| result.artifact(id).is_none())
                })
            })
        {
            invalid.push(format!("{} has conflicting artwork links", artifact.name));
        }
    }
    if let Err(error) = crate::authoring::validate_associations(&result) {
        invalid.push(error);
    }
    if !invalid.is_empty() {
        return Err(Conflict {
            fields: invalid
                .into_iter()
                .map(|label| ConflictingField {
                    label,
                    local: String::new(),
                    saved: String::new(),
                })
                .collect(),
            requires_reload: true,
        });
    }
    if resolution == Resolution::Check && !m.conflicts.is_empty() {
        return Err(Conflict {
            fields: m.conflicts,
            requires_reload: false,
        });
    }
    Ok(result)
}
