use super::super::solver::{LayoutSolver, SolverNodeId, SolverStyle};
use super::artifacts::{ArtifactProof, ArtifactStore, LayoutArtifact, LayoutArtifactKey};
use super::*;
use crate::px;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TestArtifactKey(&'static str);

fn test_artifact_key(key: TestArtifactKey) -> LayoutArtifactKey {
    LayoutArtifactKey::new(
        key,
        |_key, known_dimensions, available_space, _window, _cx| {
            let width = known_dimensions
                .width
                .unwrap_or(match available_space.width {
                    AvailableSpace::Definite(width) => width,
                    AvailableSpace::MinContent | AvailableSpace::MaxContent => px(10.0),
                });
            let height = known_dimensions.height.unwrap_or(px(20.0));
            size(width, height)
        },
    )
}

fn test_artifact(key: TestArtifactKey, measured_width: f32) -> LayoutArtifact {
    test_artifact_with_size(key, size(px(measured_width), px(20.0)))
}

fn test_artifact_with_size(key: TestArtifactKey, measured_size: Size<Pixels>) -> LayoutArtifact {
    LayoutArtifact::new(test_artifact_key(key), measured_size)
}

fn artifact_summary(artifact: Option<LayoutArtifact>) -> Option<(LayoutArtifactKey, Size<Pixels>)> {
    artifact.map(|artifact| (artifact.key().clone(), artifact.size()))
}

fn solver_node_id() -> SolverNodeId {
    let mut solver = LayoutSolver::new();
    solver.new_leaf(SolverStyle::default())
}

fn artifact_proof(
    node_id: SolverNodeId,
    key: TestArtifactKey,
    available_width: f32,
    measured_width: f32,
) -> ArtifactProof {
    ArtifactProof {
        node_id,
        artifact_key: test_artifact_key(key),
        known_dimensions: size(None, None),
        available_space: size(
            AvailableSpace::Definite(px(available_width)),
            AvailableSpace::MaxContent,
        ),
        measured_size: size(px(measured_width), px(20.0)),
    }
}

#[test]
fn artifact_cache_key_distinguishes_solver_query() {
    let node_id = solver_node_id();
    let narrow = artifact_proof(node_id, TestArtifactKey("same facts"), 80.0, 80.0);
    let wide = artifact_proof(node_id, TestArtifactKey("same facts"), 160.0, 160.0);

    assert_ne!(narrow.cache_key(), wide.cache_key());
}

#[test]
fn artifact_cache_key_distinguishes_solver_node() {
    let mut solver = LayoutSolver::new();
    let first_node = solver.new_leaf(SolverStyle::default());
    let second_node = solver.new_leaf(SolverStyle::default());
    let first = artifact_proof(first_node, TestArtifactKey("same facts"), 80.0, 80.0);
    let second = artifact_proof(second_node, TestArtifactKey("same facts"), 80.0, 80.0);

    assert_ne!(first.cache_key(), second.cache_key());
}

#[test]
fn artifact_proof_accepts_exact_key_and_solver_result() {
    let key = TestArtifactKey("same facts");
    let proof = artifact_proof(solver_node_id(), key.clone(), 80.0, 80.0);
    let artifact = test_artifact(key, 80.0);

    proof.assert_matches_artifact(&artifact, 1.0);
}

#[test]
fn artifact_proof_compares_solver_snapped_size() {
    let key = TestArtifactKey("same facts");
    let mut proof = artifact_proof(solver_node_id(), key.clone(), 0.01, 0.5);
    proof.measured_size.height = px(0.5);
    let artifact = test_artifact_with_size(key, size(px(0.01), px(0.01)));

    proof.assert_matches_artifact(&artifact, 2.0);
}

#[test]
#[should_panic(expected = "current artifact should match the exact solver query measured size")]
fn artifact_proof_rejects_same_key_with_different_solver_result() {
    let key = TestArtifactKey("same facts");
    let proof = artifact_proof(solver_node_id(), key.clone(), 80.0, 80.0);
    let artifact = test_artifact(key, 81.0);

    proof.assert_matches_artifact(&artifact, 1.0);
}

#[test]
fn artifact_store_drops_cached_query_without_current_solver_proof() {
    let mut store = ArtifactStore::new();
    let key = TestArtifactKey("same facts");
    let proof = artifact_proof(solver_node_id(), key.clone(), 80.0, 80.0);
    let cache_key = proof.cache_key();
    let artifact = test_artifact(key, 80.0);
    let expected = Some((proof.artifact_key.clone(), size(px(80.0), px(20.0))));

    store.cache_artifact(cache_key.clone(), artifact);
    assert_eq!(
        artifact_summary(store.artifact_for_query(&cache_key)),
        expected
    );

    store.begin_frame();
    assert_eq!(
        artifact_summary(store.artifact_for_query(&cache_key)),
        expected
    );

    store.finish_frame();
    assert_eq!(artifact_summary(store.artifact_for_query(&cache_key)), None);
}

#[test]
fn artifact_store_retains_only_current_exact_query_cache() {
    let mut store = ArtifactStore::new();
    let node_id = solver_node_id();
    let narrow = artifact_proof(node_id, TestArtifactKey("same facts"), 80.0, 80.0);
    let wide = artifact_proof(node_id, TestArtifactKey("same facts"), 160.0, 160.0);
    let narrow_artifact = test_artifact(TestArtifactKey("same facts"), 80.0);
    let wide_artifact = test_artifact(TestArtifactKey("same facts"), 160.0);
    let expected_narrow = Some((narrow.artifact_key.clone(), size(px(80.0), px(20.0))));
    let expected_wide = Some((wide.artifact_key.clone(), size(px(160.0), px(20.0))));

    store.cache_artifact(narrow.cache_key(), narrow_artifact.clone());
    store.cache_artifact(wide.cache_key(), wide_artifact);
    assert_eq!(
        (
            artifact_summary(store.artifact_for_query(&narrow.cache_key())),
            artifact_summary(store.artifact_for_query(&wide.cache_key())),
        ),
        (expected_narrow.clone(), expected_wide)
    );

    store.begin_frame();
    store.record_for_query(narrow.cache_key(), &narrow_artifact);
    assert_eq!(
        (
            artifact_summary(store.current_artifact_for_query(&narrow.cache_key())),
            artifact_summary(store.current_artifact_for_query(&wide.cache_key())),
        ),
        (expected_narrow.clone(), None)
    );

    store.finish_frame();
    assert_eq!(
        (
            artifact_summary(store.artifact_for_query(&narrow.cache_key())),
            artifact_summary(store.artifact_for_query(&wide.cache_key())),
        ),
        (expected_narrow, None)
    );
}
