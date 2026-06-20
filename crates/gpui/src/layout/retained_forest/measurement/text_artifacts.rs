use super::super::AvailableSpace;
use super::super::solver::{SolverMeasureObservation, SolverNodeId};
use crate::{Pixels, Size, TextLayoutArtifact, TextMeasureKey, size};
use collections::FxHashMap;

/// GPUI-owned text artifacts selected for the current layout solve.
///
/// The solver owns whether a measured callback runs. This store owns only the
/// artifact validity facts GPUI can prove in the current solve: exact
/// query-keyed artifacts observed from callbacks or passive solver cache events.
pub(super) struct TextArtifactStore {
    current_artifacts: FxHashMap<SolverNodeId, TextLayoutArtifact>,
    current_query_artifacts: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
    pending_queries: Vec<PendingTextArtifactQuery>,
    query_cache: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
}

/// Transaction checkpoint for GPUI-owned text artifact validity.
pub(super) struct TextArtifactStoreCheckpoint {
    current_artifacts: FxHashMap<SolverNodeId, TextLayoutArtifact>,
    current_query_artifacts: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
    pending_queries: Vec<PendingTextArtifactQuery>,
    query_cache: FxHashMap<TextArtifactCacheKey, TextLayoutArtifact>,
}

#[derive(Clone)]
pub(super) struct PendingTextArtifactQuery {
    pub(super) node_id: SolverNodeId,
    pub(super) text_key: TextMeasureKey,
    pub(super) known_dimensions: Size<Option<Pixels>>,
    pub(super) available_space: Size<AvailableSpace>,
}

impl TextArtifactStore {
    pub(super) fn new() -> Self {
        Self {
            current_artifacts: FxHashMap::default(),
            current_query_artifacts: FxHashMap::default(),
            pending_queries: Vec::new(),
            query_cache: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.current_artifacts.clear();
        self.current_query_artifacts.clear();
        self.pending_queries.clear();
    }

    pub(super) fn finish_frame(&mut self) {
        self.retain_current_query_cache();
        self.current_artifacts.clear();
        self.current_query_artifacts.clear();
        self.pending_queries.clear();
    }

    pub(super) fn checkpoint(&self) -> TextArtifactStoreCheckpoint {
        TextArtifactStoreCheckpoint {
            current_artifacts: self.current_artifacts.clone(),
            current_query_artifacts: self.current_query_artifacts.clone(),
            pending_queries: self.pending_queries.clone(),
            query_cache: self.query_cache.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: TextArtifactStoreCheckpoint) {
        self.current_artifacts = checkpoint.current_artifacts;
        self.current_query_artifacts = checkpoint.current_query_artifacts;
        self.pending_queries = checkpoint.pending_queries;
        self.query_cache = checkpoint.query_cache;
    }

    pub(super) fn push_pending_query(&mut self, query: PendingTextArtifactQuery) {
        self.pending_queries.push(query);
    }

    pub(super) fn take_pending_queries(&mut self) -> Vec<PendingTextArtifactQuery> {
        std::mem::take(&mut self.pending_queries)
    }

    pub(super) fn current_artifacts(&self) -> Vec<(SolverNodeId, TextLayoutArtifact)> {
        self.current_artifacts
            .iter()
            .map(|(node_id, artifact)| (*node_id, artifact.clone()))
            .collect()
    }

    pub(super) fn has_current_artifact(&self, node_id: SolverNodeId) -> bool {
        self.current_artifacts.contains_key(&node_id)
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
        node_id: SolverNodeId,
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

    fn retain_current_query_cache(&mut self) {
        let current_queries = &self.current_query_artifacts;
        self.query_cache
            .retain(|artifact_key, _| current_queries.contains_key(artifact_key));
    }
}

impl PendingTextArtifactQuery {
    pub(super) fn from_solver_measure_observation(
        text_key: TextMeasureKey,
        observation: SolverMeasureObservation,
        scale_factor: f32,
    ) -> Self {
        let query = observation.query(scale_factor);
        Self {
            node_id: observation.node_id(),
            text_key,
            known_dimensions: query.known_dimensions,
            available_space: query.available_space,
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

/// Exact validity key for a text artifact observed through the solver.
///
/// This key is built only from the measurement query the solver passes to GPUI.
/// It does not predict or reconstruct the solver cache key; it lets GPUI avoid
/// rebuilding a paint artifact when the solver asks the same text node the same
/// question again.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct TextArtifactCacheKey {
    node_id: SolverNodeId,
    text_key: TextMeasureKey,
    known_dimensions: Size<Option<Pixels>>,
    available_space: Size<AvailableSpaceCacheKey>,
}

impl TextArtifactCacheKey {
    pub(super) fn new(
        node_id: SolverNodeId,
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
