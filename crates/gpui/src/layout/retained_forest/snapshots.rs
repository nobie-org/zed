//! Exact solved-root snapshots for retained layout.
//!
//! A snapshot is GPUI-owned solved geometry plus text artifacts for one legal
//! retained root input. It is not a Taffy cache entry and does not expose or
//! reconstruct Taffy's measurement-cache key.

use super::super::{AvailableSpace, RetainedLayoutRootId};
use super::{MeasuredLayoutKind, RetainedLayoutOccurrence};
use crate::{Size, TextLayoutArtifact, size};
use collections::{FxHashMap, FxHashSet};
use taffy::tree::{Layout, NodeId};

/// Root-owned solved layout snapshots.
#[derive(Clone)]
pub(super) struct RootSnapshotStore {
    snapshots: FxHashMap<RetainedLayoutRootId, RootLayoutSnapshot>,
}

/// Transaction checkpoint for solved-root snapshots.
pub(super) struct RootSnapshotStoreCheckpoint {
    snapshots: FxHashMap<RetainedLayoutRootId, RootLayoutSnapshot>,
}

/// Data replayed for one exact root snapshot hit.
#[derive(Clone)]
pub(super) struct RootSnapshotReplay {
    pub(super) layouts: FxHashMap<NodeId, Layout>,
    pub(super) text_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
}

#[derive(Clone)]
struct RootLayoutSnapshot {
    occurrence: RetainedLayoutOccurrence,
    input: SnapshotRootInput,
    layouts: FxHashMap<NodeId, Layout>,
    text_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SnapshotRootInput {
    available_space: Size<SnapshotAvailableSpace>,
    scale_factor_bits: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SnapshotAvailableSpace {
    Definite(u32),
    #[default]
    MinContent,
    MaxContent,
}

impl RootSnapshotStore {
    pub(super) fn new() -> Self {
        Self {
            snapshots: FxHashMap::default(),
        }
    }

    pub(super) fn checkpoint(&self) -> RootSnapshotStoreCheckpoint {
        RootSnapshotStoreCheckpoint {
            snapshots: self.snapshots.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: RootSnapshotStoreCheckpoint) {
        self.snapshots = checkpoint.snapshots;
    }

    pub(super) fn retain_roots(
        &mut self,
        root_ids: impl IntoIterator<Item = RetainedLayoutRootId>,
    ) {
        let retained = root_ids.into_iter().collect::<FxHashSet<_>>();
        self.snapshots
            .retain(|root_id, _| retained.contains(root_id));
    }

    pub(super) fn replay_for(
        &self,
        root_id: RetainedLayoutRootId,
        occurrence: &RetainedLayoutOccurrence,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
    ) -> Option<RootSnapshotReplay> {
        let snapshot = self.snapshots.get(&root_id)?;
        if snapshot.input != SnapshotRootInput::new(available_space, scale_factor) {
            return None;
        }
        if snapshot.occurrence != *occurrence {
            return None;
        }

        Some(RootSnapshotReplay {
            layouts: snapshot.layouts.clone(),
            text_artifacts: snapshot.text_artifacts.clone(),
        })
    }

    pub(super) fn replay_matching_subtrees_for(
        &self,
        root_id: RetainedLayoutRootId,
        occurrence: &RetainedLayoutOccurrence,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        current_layouts: &FxHashMap<NodeId, Layout>,
    ) -> RootSnapshotReplay {
        let Some(snapshot) = self.snapshots.get(&root_id) else {
            return RootSnapshotReplay::default();
        };
        if snapshot.input != SnapshotRootInput::new(available_space, scale_factor) {
            return RootSnapshotReplay::default();
        }

        let mut previous_by_node = FxHashMap::default();
        collect_occurrences_by_node(&snapshot.occurrence, &mut previous_by_node);

        let mut replay = RootSnapshotReplay::default();
        collect_matching_subtree_replay(
            occurrence,
            &previous_by_node,
            &snapshot.layouts,
            &snapshot.text_artifacts,
            current_layouts,
            &mut replay,
        );
        replay
    }

    pub(super) fn store(
        &mut self,
        root_id: RetainedLayoutRootId,
        occurrence: &RetainedLayoutOccurrence,
        available_space: Size<AvailableSpace>,
        scale_factor: f32,
        layouts: FxHashMap<NodeId, Layout>,
        text_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
    ) {
        if !snapshot_eligible(occurrence) {
            self.snapshots.remove(&root_id);
            return;
        }

        debug_assert_snapshot_complete(occurrence, &layouts, &text_artifacts);
        self.snapshots.insert(
            root_id,
            RootLayoutSnapshot {
                occurrence: occurrence.clone(),
                input: SnapshotRootInput::new(available_space, scale_factor),
                layouts,
                text_artifacts,
            },
        );
    }
}

impl SnapshotRootInput {
    fn new(available_space: Size<AvailableSpace>, scale_factor: f32) -> Self {
        Self {
            available_space: size(
                SnapshotAvailableSpace::from(available_space.width),
                SnapshotAvailableSpace::from(available_space.height),
            ),
            scale_factor_bits: scale_factor.to_bits(),
        }
    }
}

impl Default for RootSnapshotReplay {
    fn default() -> Self {
        Self {
            layouts: FxHashMap::default(),
            text_artifacts: FxHashMap::default(),
        }
    }
}

impl From<AvailableSpace> for SnapshotAvailableSpace {
    fn from(value: AvailableSpace) -> Self {
        match value {
            AvailableSpace::Definite(pixels) => Self::Definite(pixels.0.to_bits()),
            AvailableSpace::MinContent => Self::MinContent,
            AvailableSpace::MaxContent => Self::MaxContent,
        }
    }
}

fn snapshot_eligible(occurrence: &RetainedLayoutOccurrence) -> bool {
    if matches!(
        occurrence.facts.measured_kind.as_ref(),
        Some(MeasuredLayoutKind::Opaque)
    ) {
        return false;
    }

    occurrence.children.iter().all(snapshot_eligible)
}

fn debug_assert_snapshot_complete(
    occurrence: &RetainedLayoutOccurrence,
    layouts: &FxHashMap<NodeId, Layout>,
    text_artifacts: &FxHashMap<NodeId, TextLayoutArtifact>,
) {
    debug_assert!(
        layouts.contains_key(&occurrence.node_id),
        "root snapshot should include every retained node layout"
    );

    if let Some(MeasuredLayoutKind::Text(expected_key)) = occurrence.facts.measured_kind.as_ref() {
        if let Some(artifact) = text_artifacts.get(&occurrence.node_id) {
            debug_assert_eq!(
                artifact.key(),
                expected_key,
                "root snapshot text artifact should match retained text key"
            );
        }
    }

    for child in &occurrence.children {
        debug_assert_snapshot_complete(child, layouts, text_artifacts);
    }
}

fn collect_occurrences_by_node<'a>(
    occurrence: &'a RetainedLayoutOccurrence,
    occurrences: &mut FxHashMap<NodeId, &'a RetainedLayoutOccurrence>,
) {
    occurrences.insert(occurrence.node_id, occurrence);
    for child in &occurrence.children {
        collect_occurrences_by_node(child, occurrences);
    }
}

fn collect_matching_subtree_replay(
    occurrence: &RetainedLayoutOccurrence,
    previous_by_node: &FxHashMap<NodeId, &RetainedLayoutOccurrence>,
    snapshot_layouts: &FxHashMap<NodeId, Layout>,
    snapshot_text_artifacts: &FxHashMap<NodeId, TextLayoutArtifact>,
    current_layouts: &FxHashMap<NodeId, Layout>,
    replay: &mut RootSnapshotReplay,
) {
    if let Some(previous) = previous_by_node.get(&occurrence.node_id) {
        let current_layout = current_layouts.get(&occurrence.node_id);
        let snapshot_layout = snapshot_layouts.get(&occurrence.node_id);
        if *previous == occurrence && current_layout == snapshot_layout {
            collect_snapshot_subtree(
                occurrence,
                snapshot_layouts,
                snapshot_text_artifacts,
                replay,
            );
            return;
        }
    }

    for child in &occurrence.children {
        collect_matching_subtree_replay(
            child,
            previous_by_node,
            snapshot_layouts,
            snapshot_text_artifacts,
            current_layouts,
            replay,
        );
    }
}

fn collect_snapshot_subtree(
    occurrence: &RetainedLayoutOccurrence,
    snapshot_layouts: &FxHashMap<NodeId, Layout>,
    snapshot_text_artifacts: &FxHashMap<NodeId, TextLayoutArtifact>,
    replay: &mut RootSnapshotReplay,
) {
    let layout = snapshot_layouts
        .get(&occurrence.node_id)
        .expect("root snapshot should include every retained node layout");
    replay.layouts.insert(occurrence.node_id, layout.clone());

    if let Some(artifact) = snapshot_text_artifacts.get(&occurrence.node_id) {
        replay
            .text_artifacts
            .insert(occurrence.node_id, artifact.clone());
    }

    for child in &occurrence.children {
        collect_snapshot_subtree(child, snapshot_layouts, snapshot_text_artifacts, replay);
    }
}
