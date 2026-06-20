use super::super::AvailableSpace;
use crate::{Pixels, Size, TextLayoutArtifact, TextMeasureKey, size};
use collections::{FxHashMap, FxHashSet};
use taffy::{
    LayoutCacheEntry, LayoutCacheEntryId, RunMode, style::AvailableSpace as TaffyAvailableSpace,
    tree::NodeId,
};

/// GPUI-owned text artifacts selected for the current layout solve.
///
/// Taffy owns whether a measured callback runs. This store owns only the
/// artifact validity facts GPUI can prove: exact query-keyed artifacts observed
/// from callbacks or passive Taffy cache events.
pub(super) struct TextArtifactStore {
    current_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
    current_query_artifacts: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
    current_subtree_entries: FxHashSet<TextSubtreeCacheKey>,
    pending_queries: Vec<PendingTextArtifactQuery>,
    query_cache: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
    subtree_cache: FxHashMap<TextSubtreeCacheKey, Vec<TextSubtreeArtifact>>,
}

/// Transaction checkpoint for GPUI-owned text artifact validity.
pub(super) struct TextArtifactStoreCheckpoint {
    current_artifacts: FxHashMap<NodeId, TextLayoutArtifact>,
    current_query_artifacts: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
    current_subtree_entries: FxHashSet<TextSubtreeCacheKey>,
    pending_queries: Vec<PendingTextArtifactQuery>,
    query_cache: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
    subtree_cache: FxHashMap<TextSubtreeCacheKey, Vec<TextSubtreeArtifact>>,
}

#[derive(Clone)]
pub(super) struct PendingTextArtifactQuery {
    pub(super) node_id: NodeId,
    pub(super) text_key: TextMeasureKey,
    pub(super) known_dimensions: Size<Option<Pixels>>,
    pub(super) available_space: Size<AvailableSpace>,
    pub(super) expected_taffy_size: Size<f32>,
}

impl TextArtifactStore {
    pub(super) fn new() -> Self {
        Self {
            current_artifacts: FxHashMap::default(),
            current_query_artifacts: FxHashMap::default(),
            current_subtree_entries: FxHashSet::default(),
            pending_queries: Vec::new(),
            query_cache: FxHashMap::default(),
            subtree_cache: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.current_artifacts.clear();
        self.current_query_artifacts.clear();
        self.current_subtree_entries.clear();
        self.pending_queries.clear();
    }

    pub(super) fn finish_frame(&mut self) {
        self.retain_current_query_cache();
        self.retain_current_subtree_cache();
        self.current_artifacts.clear();
        self.current_query_artifacts.clear();
        self.current_subtree_entries.clear();
        self.pending_queries.clear();
    }

    pub(super) fn checkpoint(&self) -> TextArtifactStoreCheckpoint {
        TextArtifactStoreCheckpoint {
            current_artifacts: self.current_artifacts.clone(),
            current_query_artifacts: self.current_query_artifacts.clone(),
            current_subtree_entries: self.current_subtree_entries.clone(),
            pending_queries: self.pending_queries.clone(),
            query_cache: self.query_cache.clone(),
            subtree_cache: self.subtree_cache.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: TextArtifactStoreCheckpoint) {
        self.current_artifacts = checkpoint.current_artifacts;
        self.current_query_artifacts = checkpoint.current_query_artifacts;
        self.current_subtree_entries = checkpoint.current_subtree_entries;
        self.pending_queries = checkpoint.pending_queries;
        self.query_cache = checkpoint.query_cache;
        self.subtree_cache = checkpoint.subtree_cache;
    }

    pub(super) fn push_pending_query(&mut self, query: PendingTextArtifactQuery) {
        self.pending_queries.push(query);
    }

    pub(super) fn take_pending_queries(&mut self) -> Vec<PendingTextArtifactQuery> {
        std::mem::take(&mut self.pending_queries)
    }

    pub(super) fn has_current_paint_artifact(&self, node_id: NodeId) -> bool {
        self.current_artifacts.contains_key(&node_id)
    }

    pub(super) fn current_artifacts(&self) -> Vec<(NodeId, TextLayoutArtifact)> {
        self.current_artifacts
            .iter()
            .map(|(node_id, artifact)| (*node_id, artifact.clone()))
            .collect()
    }

    pub(super) fn artifact_for_query(
        &self,
        cache_key: &TextArtifactCacheKey,
    ) -> Option<TextLayoutArtifact> {
        self.query_cache.get(cache_key).cloned()
    }

    pub(super) fn current_artifact_for_query(
        &self,
        cache_key: &TextArtifactCacheKey,
    ) -> Option<TextLayoutArtifact> {
        self.current_query_artifacts.get(cache_key).cloned()
    }

    pub(super) fn record_for_query(
        &mut self,
        node_id: NodeId,
        cache_key: TextArtifactCacheKey,
        artifact: &TextLayoutArtifact,
    ) {
        if let Some(existing) = self
            .current_query_artifacts
            .insert(cache_key, artifact.clone())
        {
            assert_eq!(
                existing.key(),
                artifact.key(),
                "same text query should not select artifacts for different text facts"
            );
            assert_eq!(
                existing.size(),
                artifact.size(),
                "same text query should not select artifacts with different sizes"
            );
        } else {
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

    pub(super) fn hydrate_from_subtree_cache_hit(
        &mut self,
        entry: LayoutCacheEntry,
        mut current_text_key: impl FnMut(NodeId) -> Option<TextMeasureKey>,
    ) {
        let Some(cache_key) = TextSubtreeCacheKey::from_final_layout_entry(entry) else {
            return;
        };
        let Some(artifacts) = self.subtree_cache.get(&cache_key).cloned() else {
            return;
        };

        for artifact in artifacts {
            let current_key = current_text_key(artifact.node_id).unwrap_or_else(|| {
                panic!(
                    "Taffy subtree cache-hit text artifact should refer to a current text measurement"
                )
            });
            assert_eq!(
                artifact.text_key, current_key,
                "Taffy subtree cache-hit text artifact should match the current text measure key"
            );
            assert_eq!(
                artifact.artifact.key(),
                &current_key,
                "Taffy subtree cache-hit artifact should match the current text measure key"
            );
            if let Some(existing) = self.current_artifacts.get(&artifact.node_id) {
                assert_eq!(
                    existing.key(),
                    &current_key,
                    "current text artifact should match the current text measure key"
                );
                assert_eq!(
                    existing.size(),
                    artifact.artifact.size(),
                    "current text artifact should match the subtree cache-hit artifact size"
                );
            } else {
                self.current_artifacts
                    .insert(artifact.node_id, artifact.artifact);
            }
        }
        self.current_subtree_entries.insert(cache_key);
    }

    pub(super) fn store_subtree_cache_entry(
        &mut self,
        entry: LayoutCacheEntry,
        text_descendants: &[NodeId],
        mut current_text_key: impl FnMut(NodeId) -> Option<TextMeasureKey>,
    ) {
        let Some(cache_key) = TextSubtreeCacheKey::from_final_layout_entry(entry) else {
            return;
        };

        let mut artifacts = Vec::with_capacity(text_descendants.len());
        for node_id in text_descendants {
            let current_key = current_text_key(*node_id).unwrap_or_else(|| {
                panic!("text descendant should have a current text measurement")
            });
            let Some(artifact) = self.current_artifacts.get(node_id) else {
                // Hidden text can be present in the current retained tree
                // without a measurement query or paint artifact. No artifact
                // means no current proof to persist for a future subtree hit.
                continue;
            };
            assert_eq!(
                artifact.key(),
                &current_key,
                "stored Taffy subtree text artifact should match the current text measure key"
            );
            artifacts.push(TextSubtreeArtifact {
                node_id: *node_id,
                text_key: current_key,
                artifact: artifact.clone(),
            });
        }

        self.current_subtree_entries.insert(cache_key.clone());
        if artifacts.is_empty() {
            self.subtree_cache.remove(&cache_key);
        } else {
            self.subtree_cache.insert(cache_key, artifacts);
        }
    }

    pub(super) fn clear_subtree_cache_entry(&mut self, node_id: NodeId) {
        self.subtree_cache
            .retain(|cache_key, _| cache_key.node_id != node_id);
    }

    fn retain_current_query_cache(&mut self) {
        let current_queries = &self.current_query_artifacts;
        self.query_cache
            .retain(|artifact_key, _| current_queries.contains_key(artifact_key));
    }

    fn retain_current_subtree_cache(&mut self) {
        let current_subtree_entries = &self.current_subtree_entries;
        self.subtree_cache
            .retain(|cache_key, _| current_subtree_entries.contains(cache_key));
    }
}

impl PendingTextArtifactQuery {
    pub(super) fn from_taffy_entry(
        node_id: NodeId,
        text_key: TextMeasureKey,
        entry: LayoutCacheEntry,
        scale_factor: f32,
    ) -> Self {
        let input = entry.requested_input();
        Self {
            node_id,
            text_key,
            known_dimensions: size(
                input
                    .known_dimensions
                    .width
                    .map(|value| Pixels(value / scale_factor)),
                input
                    .known_dimensions
                    .height
                    .map(|value| Pixels(value / scale_factor)),
            ),
            available_space: size(
                available_space_from_taffy(input.available_space.width, scale_factor),
                available_space_from_taffy(input.available_space.height, scale_factor),
            ),
            expected_taffy_size: size(
                entry.returned_output().size.width,
                entry.returned_output().size.height,
            ),
        }
    }

    pub(super) fn cache_key(&self) -> TextArtifactCacheKey {
        TextArtifactCacheKey::new(
            self.node_id,
            self.text_key.clone(),
            self.known_dimensions,
            self.available_space,
        )
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TextSubtreeCacheKey {
    node_id: NodeId,
    entry_id: LayoutCacheEntryId,
}

impl TextSubtreeCacheKey {
    fn from_final_layout_entry(entry: LayoutCacheEntry) -> Option<Self> {
        (entry.requested_input().run_mode == RunMode::PerformLayout).then_some(Self {
            node_id: entry.node_id(),
            entry_id: entry.entry_id(),
        })
    }
}

#[derive(Clone)]
struct TextSubtreeArtifact {
    node_id: NodeId,
    text_key: TextMeasureKey,
    artifact: TextLayoutArtifact,
}

/// Exact validity key for a text artifact observed through Taffy.
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
