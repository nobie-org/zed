use super::super::AvailableSpace;
use crate::{Pixels, Size, TextLayoutArtifact, TextMeasureKey, size};
use collections::{FxHashMap, FxHashSet};
use taffy::{LayoutCacheEntry, style::AvailableSpace as TaffyAvailableSpace, tree::NodeId};

/// GPUI-owned text artifacts selected for the current layout solve.
///
/// Taffy owns whether a measured callback runs. This store owns only the
/// artifact validity facts GPUI can prove: exact query-keyed artifacts observed
/// from callbacks/cache events, plus unchanged-node retained replays registered
/// by the forest.
pub(super) struct TextArtifactStore {
    current_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
    current_queries: FxHashSet<TextArtifactCacheKey>,
    replay_candidates: FxHashMap<NodeId, TextMeasureKey>,
    retained_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
    query_cache: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
}

/// Transaction checkpoint for GPUI-owned text artifact validity.
pub(super) struct TextArtifactStoreCheckpoint {
    current_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
    current_queries: FxHashSet<TextArtifactCacheKey>,
    replay_candidates: FxHashMap<NodeId, TextMeasureKey>,
    retained_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
    query_cache: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
}

impl TextArtifactStore {
    pub(super) fn new() -> Self {
        Self {
            current_artifacts: FxHashMap::default(),
            current_queries: FxHashSet::default(),
            replay_candidates: FxHashMap::default(),
            retained_artifacts: FxHashMap::default(),
            query_cache: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.current_artifacts.clear();
        self.current_queries.clear();
        self.replay_candidates.clear();
    }

    pub(super) fn finish_frame(&mut self, current_text_nodes: &FxHashSet<NodeId>) {
        self.retain_current_query_cache();
        self.retain_current_artifacts_by_node(current_text_nodes);
        self.current_artifacts.clear();
        self.current_queries.clear();
        self.replay_candidates.clear();
    }

    pub(super) fn checkpoint(&self) -> TextArtifactStoreCheckpoint {
        TextArtifactStoreCheckpoint {
            current_artifacts: self.current_artifacts.clone(),
            current_queries: self.current_queries.clone(),
            replay_candidates: self.replay_candidates.clone(),
            retained_artifacts: self.retained_artifacts.clone(),
            query_cache: self.query_cache.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: TextArtifactStoreCheckpoint) {
        self.current_artifacts = checkpoint.current_artifacts;
        self.current_queries = checkpoint.current_queries;
        self.replay_candidates = checkpoint.replay_candidates;
        self.retained_artifacts = checkpoint.retained_artifacts;
        self.query_cache = checkpoint.query_cache;
    }

    pub(super) fn register_replay(&mut self, node_id: NodeId, expected_key: &TextMeasureKey) {
        self.replay_candidates.insert(node_id, expected_key.clone());
    }

    pub(super) fn take_replay_candidates(&mut self) -> FxHashMap<NodeId, TextMeasureKey> {
        std::mem::take(&mut self.replay_candidates)
    }

    pub(super) fn has_current_artifact(&self, node_id: NodeId) -> bool {
        self.current_artifacts.contains_key(&node_id)
    }

    pub(super) fn retained_artifact(&self, node_id: NodeId) -> Option<TextLayoutArtifact> {
        self.retained_artifacts.get(&node_id).cloned()
    }

    pub(super) fn insert_current_artifact(
        &mut self,
        node_id: NodeId,
        artifact: TextLayoutArtifact,
    ) {
        self.current_artifacts.insert(node_id, artifact);
    }

    pub(super) fn current_artifacts(&self) -> Vec<(NodeId, TextLayoutArtifact)> {
        self.current_artifacts
            .iter()
            .map(|(node_id, artifact)| (*node_id, artifact.clone()))
            .collect()
    }

    pub(super) fn current_artifact(&self, node_id: NodeId) -> Option<TextLayoutArtifact> {
        self.current_artifacts.get(&node_id).cloned()
    }

    pub(super) fn artifact_for_query(
        &self,
        cache_key: &TextArtifactCacheKey,
    ) -> Option<TextLayoutArtifact> {
        self.query_cache.get(cache_key).cloned()
    }

    pub(super) fn record_for_query(
        &mut self,
        node_id: NodeId,
        cache_key: TextArtifactCacheKey,
        artifact: &TextLayoutArtifact,
    ) {
        if self.current_queries.insert(cache_key) {
            self.current_artifacts.insert(node_id, artifact.clone());
        }
    }

    pub(super) fn cache_artifact(
        &mut self,
        cache_key: TextArtifactCacheKey,
        artifact: TextLayoutArtifact,
    ) {
        self.query_cache.insert(cache_key, artifact);
    }

    fn retain_current_query_cache(&mut self) {
        let current_queries = &self.current_queries;
        self.query_cache
            .retain(|artifact_key, _| current_queries.contains(artifact_key));
    }

    fn retain_current_artifacts_by_node(&mut self, current_text_nodes: &FxHashSet<NodeId>) {
        self.retained_artifacts.retain(|node_id, _| {
            current_text_nodes.contains(node_id) && self.current_artifacts.contains_key(node_id)
        });
        for (node_id, artifact) in &self.current_artifacts {
            self.retained_artifacts.insert(*node_id, artifact.clone());
        }
    }
}

/// Exact validity key for a retained text artifact observed through Taffy.
///
/// This key is built only from the measurement query Taffy passes to GPUI. It
/// does not predict or reconstruct Taffy's cache key; it lets GPUI avoid
/// rebuilding a paint artifact when Taffy asks the same text node the same
/// question again.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TextArtifactCacheKey {
    node_id: NodeId,
    text_key: TextMeasureKey,
    known_dimensions: Size<Option<Pixels>>,
    available_space: Size<AvailableSpaceCacheKey>,
}

impl TextArtifactCacheKey {
    pub(super) fn new(
        node_id: NodeId,
        text_key: TextMeasureKey,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
    ) -> Self {
        Self {
            node_id,
            text_key,
            known_dimensions,
            available_space: size(
                AvailableSpaceCacheKey::from(available_space.width),
                AvailableSpaceCacheKey::from(available_space.height),
            ),
        }
    }

    pub(super) fn from_taffy_entry(
        node_id: NodeId,
        text_key: TextMeasureKey,
        entry: LayoutCacheEntry,
        scale_factor: f32,
    ) -> Self {
        let input = entry.requested_input();
        Self::new(
            node_id,
            text_key,
            size(
                input
                    .known_dimensions
                    .width
                    .map(|value| Pixels(value / scale_factor)),
                input
                    .known_dimensions
                    .height
                    .map(|value| Pixels(value / scale_factor)),
            ),
            size(
                available_space_from_taffy(input.available_space.width, scale_factor),
                available_space_from_taffy(input.available_space.height, scale_factor),
            ),
        )
    }
}

fn available_space_from_taffy(value: TaffyAvailableSpace, scale_factor: f32) -> AvailableSpace {
    match value {
        TaffyAvailableSpace::Definite(value) => {
            AvailableSpace::Definite(Pixels(value / scale_factor))
        }
        TaffyAvailableSpace::MinContent => AvailableSpace::MinContent,
        TaffyAvailableSpace::MaxContent => AvailableSpace::MaxContent,
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
enum AvailableSpaceCacheKey {
    Definite(Pixels),
    #[default]
    MinContent,
    MaxContent,
}

impl From<AvailableSpace> for AvailableSpaceCacheKey {
    fn from(value: AvailableSpace) -> Self {
        match value {
            AvailableSpace::Definite(pixels) => Self::Definite(pixels),
            AvailableSpace::MinContent => Self::MinContent,
            AvailableSpace::MaxContent => Self::MaxContent,
        }
    }
}
