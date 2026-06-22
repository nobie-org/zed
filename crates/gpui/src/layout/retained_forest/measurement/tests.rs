use super::super::solver::{LayoutSolver, SolverNodeId, SolverStyle};
use super::text_artifacts::TextArtifactProof;
use super::*;
use crate::{SharedString, TextLayoutArtifact, TextStyle, px};

fn text_measure_key(text: &'static str) -> TextMeasureKey {
    let text = SharedString::new_static(text);
    let text_style = TextStyle::default();
    TextMeasureKey::new(
        text.clone(),
        vec![text_style.to_run(text.len())],
        &text_style,
        px(16.0),
        px(20.0),
        1.0,
        0,
    )
}

fn solver_node_id() -> SolverNodeId {
    let mut solver = LayoutSolver::new();
    solver.new_leaf(SolverStyle::default())
}

fn text_artifact_proof(
    node_id: SolverNodeId,
    text_key: TextMeasureKey,
    available_width: f32,
    measured_width: f32,
) -> TextArtifactProof {
    TextArtifactProof {
        node_id,
        text_key,
        known_dimensions: size(None, None),
        available_space: size(
            AvailableSpace::Definite(px(available_width)),
            AvailableSpace::MaxContent,
        ),
        measured_size: size(px(measured_width), px(20.0)),
    }
}

#[test]
fn text_artifact_cache_key_distinguishes_solver_query() {
    let node_id = solver_node_id();
    let key = text_measure_key("same text");
    let narrow = text_artifact_proof(node_id, key.clone(), 80.0, 80.0);
    let wide = text_artifact_proof(node_id, key, 160.0, 160.0);

    assert_ne!(narrow.cache_key(), wide.cache_key());
}

#[test]
fn text_artifact_cache_key_distinguishes_solver_node() {
    let mut solver = LayoutSolver::new();
    let first_node = solver.new_leaf(SolverStyle::default());
    let second_node = solver.new_leaf(SolverStyle::default());
    let key = text_measure_key("same text");
    let first = text_artifact_proof(first_node, key.clone(), 80.0, 80.0);
    let second = text_artifact_proof(second_node, key, 80.0, 80.0);

    assert_ne!(first.cache_key(), second.cache_key());
}

#[test]
fn text_artifact_proof_accepts_exact_key_and_solver_result() {
    let key = text_measure_key("same text");
    let proof = text_artifact_proof(solver_node_id(), key.clone(), 80.0, 80.0);
    let artifact = TextLayoutArtifact::for_tests(key, size(px(80.0), px(20.0)));

    assert_text_artifact_matches_proof(&artifact, &proof, 1.0);
}

#[test]
fn text_artifact_proof_compares_solver_snapped_size() {
    let key = text_measure_key("same text");
    let mut proof = text_artifact_proof(solver_node_id(), key.clone(), 0.01, 0.5);
    proof.measured_size.height = px(1.0);
    let artifact = TextLayoutArtifact::for_tests(key, size(px(0.01), px(1.0)));

    assert_text_artifact_matches_proof(&artifact, &proof, 2.0);
}

#[test]
#[should_panic(
    expected = "current text artifact should match the exact solver query measured size"
)]
fn text_artifact_proof_rejects_same_key_with_different_solver_result() {
    let key = text_measure_key("same text");
    let proof = text_artifact_proof(solver_node_id(), key.clone(), 80.0, 80.0);
    let artifact = TextLayoutArtifact::for_tests(key, size(px(81.0), px(20.0)));

    assert_text_artifact_matches_proof(&artifact, &proof, 1.0);
}
