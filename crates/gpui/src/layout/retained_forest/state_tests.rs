use super::super::{LayoutId, RetainedLayoutRootId, RetainedLayoutRootSite};
use super::{
    committed::CommittedLayoutState,
    facts::{CurrentLayoutNodeFacts, CurrentLayoutNodeKind},
    frame::CurrentLayoutFactsLog,
    geometry::FrameLayoutOutput,
    measurement::{
        CurrentMeasurement, LayoutMeasureContext, MeasuredLayoutFacts, MeasurementStore,
        PureSizeMeasure,
    },
    node::{RetainedLayoutNode, RetainedLayoutNodeKind},
    root_slots::RootSlots,
    roots::RootRegistry,
    solver::{LayoutSolver, SolverNodeId, SolverStyle},
    work::{RetainedLayoutMissWork, RetainedLayoutWork, RetainedWorkState},
};
use crate::{
    AvailableSpace, Bounds, ElementId, GlobalElementId, Pixels, Size, TestAppContext, px, size,
};
use hegel::generators;
use stacksafe::StackSafe;
use std::{collections::HashMap, sync::Arc};

fn hegel_settings(test_cases: u64) -> hegel::Settings {
    hegel::Settings::new().test_cases(test_cases)
}

fn draw_u8(tc: &hegel::TestCase, min: u8, max: u8) -> u8 {
    tc.draw(generators::integers::<u8>().min_value(min).max_value(max))
}

fn draw_usize(tc: &hegel::TestCase, min: usize, max: usize) -> usize {
    tc.draw(
        generators::integers::<usize>()
            .min_value(min)
            .max_value(max),
    )
}

fn global_id(id: u8) -> GlobalElementId {
    GlobalElementId(Arc::from([ElementId::Integer(id.into())]))
}

fn unmeasured_facts(children: Vec<LayoutId>) -> CurrentLayoutNodeFacts {
    CurrentLayoutNodeFacts {
        global_id: None,
        style: SolverStyle::default(),
        kind: CurrentLayoutNodeKind::Unmeasured { children },
    }
}

fn measured_facts(measured_facts: MeasuredLayoutFacts) -> CurrentLayoutNodeFacts {
    CurrentLayoutNodeFacts {
        global_id: None,
        style: SolverStyle::default(),
        kind: CurrentLayoutNodeKind::Measured(measured_facts),
    }
}

fn new_test_node(solver: &mut LayoutSolver) -> SolverNodeId {
    solver.new_leaf(SolverStyle::default())
}

fn new_node(node_id: SolverNodeId) -> RetainedLayoutNode {
    RetainedLayoutNode {
        node_id,
        identity: None,
        style: SolverStyle::default(),
        kind: RetainedLayoutNodeKind::Unmeasured {
            children: Vec::new(),
        },
    }
}

fn producer_context(width: f32, height: f32) -> LayoutMeasureContext {
    LayoutMeasureContext::Size(StackSafe::new(Box::new(move |_, _, _| {
        size(px(width), px(height))
    })))
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ModelRootKey {
    Global { site: u8, id: u8 },
}

fn root_site(site: u8) -> RetainedLayoutRootSite {
    match site {
        0 => root_site_zero(),
        1 => root_site_one(),
        _ => root_site_two(),
    }
}

fn root_site_zero() -> RetainedLayoutRootSite {
    RetainedLayoutRootSite::caller(core::panic::Location::caller())
}

fn root_site_one() -> RetainedLayoutRootSite {
    RetainedLayoutRootSite::caller(core::panic::Location::caller())
}

fn root_site_two() -> RetainedLayoutRootSite {
    RetainedLayoutRootSite::caller(core::panic::Location::caller())
}

#[gpui::test]
fn root_registry_matches_generated_identity_model(_cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let mut registry = RootRegistry::new();
        let mut model = HashMap::<ModelRootKey, RetainedLayoutRootId>::new();
        let mut next_root_id = 0;

        let frame_count = draw_usize(&tc, 1, 8);
        for _ in 0..frame_count {
            let request_count = draw_usize(&tc, 0, 10);
            let mut actual = Vec::new();
            let mut expected = Vec::new();

            for _ in 0..request_count {
                let site = draw_u8(&tc, 0, 2);
                let root_site = root_site(site);
                let use_global = tc.draw(generators::booleans());
                if use_global {
                    let id = draw_u8(&tc, 0, 4);
                    let global = global_id(id);
                    actual.push(registry.retained_root_id(root_site, Some(&global)));

                    let root_id = *model
                        .entry(ModelRootKey::Global { site, id })
                        .or_insert_with(|| {
                            let root_id = RetainedLayoutRootId::new(next_root_id);
                            next_root_id += 1;
                            root_id
                        });
                    expected.push(root_id);
                } else {
                    actual.push(registry.retained_root_id(root_site, None));
                    expected.push(RetainedLayoutRootId::new(next_root_id));
                    next_root_id += 1;
                }
            }

            assert_eq!(actual, expected);
        }
    })
    .settings(hegel_settings(128))
    .run();
}

#[gpui::test]
fn frame_facts_checkpoint_restores_exact_facts_log(_cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let before_count = draw_usize(&tc, 0, 16);
        let after_count = draw_usize(&tc, 0, 16);
        let mut frame = CurrentLayoutFactsLog::new();
        let mut expected = Vec::new();

        for _ in 0..before_count {
            let facts = unmeasured_facts(Vec::new());
            let id = frame.push_facts(facts.clone());
            expected.push((id, facts));
        }

        let checkpoint = frame.checkpoint();
        for _ in 0..after_count {
            let _ = frame.push_facts(measured_facts(MeasuredLayoutFacts::opaque()));
        }
        frame.rollback_to_checkpoint(checkpoint);

        assert_eq!(
            expected.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            (0..before_count).map(LayoutId).collect::<Vec<_>>()
        );
        assert_eq!(
            expected
                .iter()
                .map(|(id, _)| (*id, frame.facts(*id).clone()))
                .collect::<Vec<_>>(),
            expected
        );
        let next = frame.push_facts(unmeasured_facts(Vec::new()));
        assert_eq!(next, LayoutId(before_count));
    })
    .settings(hegel_settings(128))
    .run();
}

#[gpui::test]
fn measurement_store_checkpoint_restores_producers_and_current_measurements(
    _cx: &mut TestAppContext,
) {
    hegel::Hegel::new(|tc| {
        let mut solver = LayoutSolver::new();
        let first_node = new_test_node(&mut solver);
        let second_node = new_test_node(&mut solver);
        let before_contexts = draw_usize(&tc, 0, 8);
        let after_contexts = draw_usize(&tc, 0, 8);
        let width = draw_u8(&tc, 1, 64) as f32;
        let height = draw_u8(&tc, 1, 64) as f32;
        let measure = PureSizeMeasure::content_size(size(px(width), px(height)), 1.0);
        let mut store = MeasurementStore::new();

        for index in 0..before_contexts {
            let actual = store.push_producer_context_for_tests(producer_context(index as f32, 1.0));
            assert_eq!(actual, index);
        }
        store.insert_current_measurement(first_node, CurrentMeasurement::PureSize(measure.clone()));

        let checkpoint = store.checkpoint();
        for index in 0..after_contexts {
            let _ = store.push_producer_context_for_tests(producer_context(index as f32, 2.0));
        }
        store.insert_current_measurement(second_node, CurrentMeasurement::Opaque(before_contexts));
        store.rollback_to_checkpoint(checkpoint);

        assert_eq!(
            store.current_measurement(first_node).cloned(),
            Some(CurrentMeasurement::PureSize(measure))
        );
        assert_eq!(store.current_measurement(second_node).cloned(), None);
        let next = store.push_producer_context_for_tests(producer_context(3.0, 4.0));
        assert_eq!(next, before_contexts);
    })
    .settings(hegel_settings(128))
    .run();
}

#[gpui::test]
fn committed_layout_state_checkpoint_restores_unique_current_mapping(_cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let mut solver = LayoutSolver::new();
        let before_count = draw_usize(&tc, 0, 12);
        let after_count = draw_usize(&tc, 0, 12);
        let mut state = CommittedLayoutState::new();
        let mut before = Vec::new();
        let mut after = Vec::new();

        for index in 0..before_count {
            let node = new_test_node(&mut solver);
            state.insert(LayoutId(index), node);
            before.push((LayoutId(index), node));
        }

        let checkpoint = state.checkpoint();
        for index in before_count..(before_count + after_count) {
            let node = new_test_node(&mut solver);
            state.insert(LayoutId(index), node);
            after.push((LayoutId(index), node));
        }
        state.rollback_to_checkpoint(checkpoint);

        assert_eq!(
            before
                .iter()
                .map(|(id, _)| (*id, state.try_node(*id)))
                .collect::<Vec<_>>(),
            before
                .iter()
                .map(|(id, node)| (*id, Some(*node)))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            before
                .iter()
                .map(|(_, node)| (*node, state.layout_id_for_node(*node)))
                .collect::<Vec<_>>(),
            before
                .iter()
                .map(|(id, node)| (*node, Some(*id)))
                .collect::<Vec<_>>()
        );
        assert_eq!(state.try_node(LayoutId(before_count)), None);
        for (id, node) in after {
            assert_eq!(state.try_node(id), None);
            assert_eq!(state.layout_id_for_node(node), None);
        }
        let next_node = new_test_node(&mut solver);
        state.insert(LayoutId(before_count), next_node);
        assert_eq!(state.try_node(LayoutId(before_count)), Some(next_node));
        assert_eq!(
            state.layout_id_for_node(next_node),
            Some(LayoutId(before_count))
        );
        state.clear();
        assert_eq!(state.try_node(LayoutId(before_count)), None);
        assert_eq!(state.layout_id_for_node(next_node), None);
    })
    .settings(hegel_settings(128))
    .run();
}

#[gpui::test]
#[should_panic(expected = "layout facts should appear only once in a committed layout tree")]
fn committed_layout_state_rejects_duplicate_layout_id(_cx: &mut TestAppContext) {
    let mut solver = LayoutSolver::new();
    let mut state = CommittedLayoutState::new();
    let first = new_test_node(&mut solver);
    let second = new_test_node(&mut solver);

    state.insert(LayoutId(1), first);
    state.insert(LayoutId(1), second);
}

#[gpui::test]
#[should_panic(expected = "committed solver node should map to only one current layout id")]
fn committed_layout_state_rejects_duplicate_solver_node(_cx: &mut TestAppContext) {
    let mut solver = LayoutSolver::new();
    let mut state = CommittedLayoutState::new();
    let node = new_test_node(&mut solver);

    state.insert(LayoutId(1), node);
    state.insert(LayoutId(2), node);
}

#[gpui::test]
fn root_slots_checkpoint_restores_current_roots_and_detached_removals(_cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let mut solver = LayoutSolver::new();
        let root_a = RetainedLayoutRootId::new(draw_u8(&tc, 0, 10).into());
        let root_b = RetainedLayoutRootId::new((draw_u8(&tc, 11, 20)).into());
        let new_node_a = new_node(new_test_node(&mut solver));
        let detached_a = new_node(new_test_node(&mut solver));
        let new_node_b = new_node(new_test_node(&mut solver));
        let detached_b = new_node(new_test_node(&mut solver));
        let mut slots = RootSlots::new();

        slots.insert_current_root(root_a, new_node_a.clone());
        slots.detach_subtree(detached_a.clone());
        let checkpoint = slots.checkpoint();
        slots.insert_current_root(root_b, new_node_b);
        slots.detach_subtree(detached_b);
        slots.rollback_to_checkpoint(checkpoint);
        slots.promote_current_roots();

        assert_eq!(slots.take_retained_root(root_a), Some(new_node_a));
        assert_eq!(slots.take_retained_root(root_b), None);
        assert_eq!(slots.take_detached_subtree_removals(), vec![detached_a]);
    })
    .settings(hegel_settings(128))
    .run();
}

#[gpui::test]
fn frame_layout_output_checkpoint_restores_captured_absolute_bounds(_cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let width = draw_u8(&tc, 0, 200) as f32;
        let height = draw_u8(&tc, 0, 200) as f32;
        let scale_factor = draw_u8(&tc, 1, 4) as f32;
        let available_space = size(
            super::super::AvailableSpace::Definite(px(width.max(1.0))),
            super::super::AvailableSpace::Definite(px(height.max(1.0))),
        );
        let mut solver = LayoutSolver::new();
        let child = solver.new_leaf(SolverStyle::test_with_size(width, height));
        let root = solver.new_with_children(SolverStyle::default(), &[child]);
        solver.compute_layout_with_measure(
            root,
            available_space,
            1.0,
            |_node_id, _has_measure_context, _query| size(0.0, 0.0),
        );

        let mut output = FrameLayoutOutput::new();
        let root_id = RetainedLayoutRootId::new(1);
        output.begin_solve(root_id, root, available_space, scale_factor);
        output.capture_from_solver(
            root_id,
            root,
            available_space,
            scale_factor,
            solver.capture_layout_tree(root),
            |node_id| solver.parent(node_id),
            |node_id| {
                if node_id == root {
                    Some(LayoutId(1))
                } else if node_id == child {
                    Some(LayoutId(2))
                } else {
                    None
                }
            },
        );

        let first = output
            .bounds(LayoutId(2))
            .expect("captured frame output should contain child bounds");
        let checkpoint = output.checkpoint();
        output.begin_frame();
        output.rollback_to_checkpoint(checkpoint);
        let second = output
            .bounds(LayoutId(2))
            .expect("rollback should restore captured child bounds");

        assert_eq!(second, first);
        assert_eq!(
            first,
            Bounds::new(
                crate::point(px(0.0), px(0.0)),
                Size {
                    width: Pixels(width / scale_factor),
                    height: Pixels(height / scale_factor),
                },
            )
        );
    })
    .settings(hegel_settings(128))
    .run();
}

#[gpui::test]
fn work_state_checkpoint_and_finish_return_complete_frame_counts(_cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let creates = draw_usize(&tc, 0, 12);
        let reuses = draw_usize(&tc, 0, 12);
        let removes = draw_usize(&tc, 0, 12);
        let traced_misses = draw_usize(&tc, 0, 8);
        let mut work = RetainedWorkState::new();

        for _ in 0..creates {
            work.record_create();
        }
        for _ in 0..reuses {
            work.record_reuse();
        }
        for _ in 0..removes {
            work.record_remove();
        }
        let checkpoint = work.checkpoint();
        work.record_style_update();
        work.record_child_list_update();
        work.record_no_previous_miss();
        work.rollback_to_checkpoint(checkpoint);

        for _ in 0..traced_misses {
            assert_eq!(work.should_trace_miss(traced_misses + 1), true);
            let _ = work.take_miss_trace_sample_index();
        }

        assert_eq!(
            work.finish_frame(),
            (
                RetainedLayoutWork {
                    creates: creates as u64,
                    reuses: reuses as u64,
                    style_updates: 0,
                    child_list_updates: 0,
                    measured_context_clears: 0,
                    removes: removes as u64,
                    ..RetainedLayoutWork::default()
                },
                RetainedLayoutMissWork::default(),
            )
        );
        work.begin_frame();
        assert_eq!(work.should_trace_miss(traced_misses), false);
        assert_eq!(
            work.finish_frame(),
            (
                RetainedLayoutWork::default(),
                RetainedLayoutMissWork::default()
            )
        );
    })
    .settings(hegel_settings(128))
    .run();
}
