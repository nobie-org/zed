use super::super::AvailableSpace;
use super::super::solver::SolverNodeId;
use crate::{Pixels, Size, Window, size};
use collections::FxHashMap;
use std::{
    any::{Any, TypeId},
    collections::hash_map::DefaultHasher,
    fmt,
    hash::{Hash, Hasher},
    rc::Rc,
};

type FreshArtifactMeasure = Rc<
    dyn Fn(Size<Option<Pixels>>, Size<AvailableSpace>, &mut Window, &crate::App) -> Size<Pixels>,
>;

/// Comparable identity for a measured node that also produces a GPUI artifact.
///
/// The retained layout system treats artifacts generically. Text, or any future
/// artifact-producing measured node, supplies an explicit key and a fresh-size
/// oracle at the layout facade boundary; retained measurement only compares
/// keys and exact solver queries.
#[derive(Clone)]
pub(in crate::layout) struct LayoutArtifactKey {
    type_id: TypeId,
    value: Rc<dyn Any>,
    hash: u64,
    eq: fn(&dyn Any, &dyn Any) -> bool,
    debug: fn(&dyn Any, &mut fmt::Formatter<'_>) -> fmt::Result,
    measure_for_fresh_compare: FreshArtifactMeasure,
}

impl LayoutArtifactKey {
    /// Build a layout artifact key from explicit comparable artifact facts.
    pub(in crate::layout) fn new<K>(
        key: K,
        measure_for_fresh_compare: impl Fn(
            &K,
            Size<Option<Pixels>>,
            Size<AvailableSpace>,
            &mut Window,
            &crate::App,
        ) -> Size<Pixels>
        + 'static,
    ) -> Self
    where
        K: Clone + fmt::Debug + Eq + Hash + 'static,
    {
        fn eq_key<K: Eq + 'static>(left: &dyn Any, right: &dyn Any) -> bool {
            left.downcast_ref::<K>() == right.downcast_ref::<K>()
        }

        fn debug_key<K: fmt::Debug + 'static>(
            value: &dyn Any,
            f: &mut fmt::Formatter<'_>,
        ) -> fmt::Result {
            value
                .downcast_ref::<K>()
                .expect("layout artifact key should have the expected type")
                .fmt(f)
        }

        let key = Rc::new(key);
        let mut hasher = DefaultHasher::new();
        TypeId::of::<K>().hash(&mut hasher);
        key.hash(&mut hasher);
        let hash = hasher.finish();
        let measure_key = Rc::clone(&key);
        let measure_for_fresh_compare = Rc::new(
            move |known_dimensions, available_space, window: &mut Window, cx: &crate::App| {
                measure_for_fresh_compare(
                    &measure_key,
                    known_dimensions,
                    available_space,
                    window,
                    cx,
                )
            },
        );

        Self {
            type_id: TypeId::of::<K>(),
            value: key,
            hash,
            eq: eq_key::<K>,
            debug: debug_key::<K>,
            measure_for_fresh_compare,
        }
    }

    pub(super) fn measure_for_fresh_compare(
        &self,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
        window: &mut Window,
        cx: &crate::App,
    ) -> Size<Pixels> {
        (self.measure_for_fresh_compare)(known_dimensions, available_space, window, cx)
    }

    pub(super) fn assert_matches_artifact(&self, artifact: &LayoutArtifact, message: &'static str) {
        assert_eq!(&artifact.key, self, "{message}");
    }
}

impl fmt::Debug for LayoutArtifactKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (self.debug)(self.value.as_ref(), f)
    }
}

impl PartialEq for LayoutArtifactKey {
    fn eq(&self, other: &Self) -> bool {
        self.type_id == other.type_id && (self.eq)(self.value.as_ref(), other.value.as_ref())
    }
}

impl Eq for LayoutArtifactKey {}

impl Hash for LayoutArtifactKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.type_id.hash(state);
        self.hash.hash(state);
    }
}

/// GPUI-owned measurement artifact valid for one artifact key and measured size.
///
/// Retained measurement only needs enough data to answer the same exact solver
/// query without invoking the producer again. Paint/hit-test payloads belong to
/// their owning elements, not to the layout measurement cache.
#[derive(Clone)]
pub(in crate::layout) struct LayoutArtifact {
    key: LayoutArtifactKey,
    size: Size<Pixels>,
}

impl LayoutArtifact {
    /// Record a measured size for explicit artifact facts.
    pub(in crate::layout) fn new(key: LayoutArtifactKey, size: Size<Pixels>) -> Self {
        Self { key, size }
    }

    pub(super) fn key(&self) -> &LayoutArtifactKey {
        &self.key
    }

    pub(super) fn size(&self) -> Size<Pixels> {
        self.size
    }
}

/// GPUI-owned artifacts selected by measured callbacks in the current solve.
///
/// The solver owns whether a measured callback runs. This store owns only the
/// artifact validity facts GPUI can prove when the callback actually asks GPUI
/// for an exact measured query.
pub(super) struct ArtifactStore {
    current_query_artifacts: FxHashMap<ArtifactCacheKey, LayoutArtifact>,
    query_cache: FxHashMap<ArtifactCacheKey, LayoutArtifact>,
}

/// Transaction checkpoint for GPUI-owned artifact validity.
pub(super) struct ArtifactStoreCheckpoint {
    current_query_artifacts: FxHashMap<ArtifactCacheKey, LayoutArtifact>,
    query_cache: FxHashMap<ArtifactCacheKey, LayoutArtifact>,
}

impl ArtifactStore {
    pub(super) fn new() -> Self {
        Self {
            current_query_artifacts: FxHashMap::default(),
            query_cache: FxHashMap::default(),
        }
    }

    pub(super) fn begin_frame(&mut self) {
        self.current_query_artifacts.clear();
    }

    pub(super) fn finish_frame(&mut self) {
        self.retain_current_query_cache();
        self.current_query_artifacts.clear();
    }

    pub(super) fn checkpoint(&self) -> ArtifactStoreCheckpoint {
        ArtifactStoreCheckpoint {
            current_query_artifacts: self.current_query_artifacts.clone(),
            query_cache: self.query_cache.clone(),
        }
    }

    pub(super) fn rollback_to_checkpoint(&mut self, checkpoint: ArtifactStoreCheckpoint) {
        self.current_query_artifacts = checkpoint.current_query_artifacts;
        self.query_cache = checkpoint.query_cache;
    }

    pub(super) fn artifact_for_query(
        &self,
        cache_key: &ArtifactCacheKey,
    ) -> Option<LayoutArtifact> {
        self.query_cache.get(cache_key).cloned()
    }

    pub(super) fn record_for_query(
        &mut self,
        cache_key: ArtifactCacheKey,
        artifact: &LayoutArtifact,
    ) {
        if let Some(existing) = self
            .current_query_artifacts
            .insert(cache_key, artifact.clone())
        {
            assert_eq!(
                existing.key(),
                artifact.key(),
                "same artifact query should not select artifacts for different facts"
            );
            assert_eq!(
                existing.size(),
                artifact.size(),
                "same artifact query should not select artifacts with different sizes"
            );
        }
    }

    pub(super) fn cache_artifact(&mut self, cache_key: ArtifactCacheKey, artifact: LayoutArtifact) {
        self.query_cache.insert(cache_key, artifact);
    }

    fn retain_current_query_cache(&mut self) {
        let current_queries = &self.current_query_artifacts;
        self.query_cache
            .retain(|artifact_key, _| current_queries.contains_key(artifact_key));
    }
}

/// Exact validity key for an artifact produced by a measured callback.
///
/// This key is built only from the measurement query the solver passes to GPUI.
/// It does not predict or reconstruct the solver cache key; it lets GPUI avoid
/// rebuilding a paint artifact when the solver asks the same measured node the
/// same question again.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct ArtifactCacheKey {
    node_id: SolverNodeId,
    artifact_key: LayoutArtifactKey,
    known_dimensions: Size<Option<Pixels>>,
    available_space: Size<AvailableSpaceCacheKey>,
}

impl ArtifactCacheKey {
    pub(super) fn new(
        node_id: SolverNodeId,
        artifact_key: LayoutArtifactKey,
        known_dimensions: Size<Option<Pixels>>,
        available_space: Size<AvailableSpace>,
    ) -> Self {
        Self {
            node_id,
            artifact_key,
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
