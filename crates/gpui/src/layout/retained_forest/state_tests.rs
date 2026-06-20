use super::super::{LayoutId, RetainedLayoutRootId, RetainedLayoutRootSite};
use super::{
    bounds_cache::BoundsCache,
    committed::CommittedLayoutState,
    facts::{LayoutArtifactPolicy, LayoutIntent, LayoutIntentKind},
    frame::FrameIntents,
    measurement::{
        CurrentMeasurement, LayoutMeasureContext, MeasuredLayoutFacts, MeasuredLayoutResult,
        MeasurementStore, PureSizeMeasure,
    },
    occurrence::{RetainedLayoutOccurrence, RetainedLayoutOccurrenceKind},
    root_slots::RootSlots,
    roots::RootRegistry,
    solver::{LayoutSolver, SolverNodeId, SolverStyle},
    work::{RetainedLayoutMissWork, RetainedLayoutWork, RetainedWorkState},
};
use crate::{Bounds, ElementId, GlobalElementId, Pixels, Size, TestAppContext, px, size};
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

fn draw_element_stack(tc: &hegel::TestCase) -> Vec<u8> {
    tc.draw(generators::vecs(generators::integers::<u8>().min_value(0).max_value(4)).max_size(4))
}

fn element_stack(ids: &[u8]) -> Vec<ElementId> {
    ids.iter()
        .copied()
        .map(|id| ElementId::Integer(id.into()))
        .collect()
}

fn global_id(id: u8) -> GlobalElementId {
    GlobalElementId(Arc::from([ElementId::Integer(id.into())]))
}

fn unmeasured_intent(children: Vec<LayoutId>) -> LayoutIntent {
    LayoutIntent {
        global_id: None,
        style: SolverStyle::default(),
        artifact_policy: LayoutArtifactPolicy::CanProduceArtifacts,
        kind: LayoutIntentKind::Unmeasured { children },
    }
}

fn measured_intent(measured_facts: MeasuredLayoutFacts) -> LayoutIntent {
    LayoutIntent {
        global_id: None,
        style: SolverStyle::default(),
        artifact_policy: LayoutArtifactPolicy::CanProduceArtifacts,
        kind: LayoutIntentKind::Measured(measured_facts),
    }
}

fn new_test_node(solver: &mut LayoutSolver) -> SolverNodeId {
    solver.new_leaf(SolverStyle::default())
}

fn occurrence(node_id: SolverNodeId) -> RetainedLayoutOccurrence {
    RetainedLayoutOccurrence {
        node_id,
        identity: None,
        style: SolverStyle::default(),
        kind: RetainedLayoutOccurrenceKind::Unmeasured {
            children: Vec::new(),
        },
    }
}

fn producer_context(width: f32, height: f32) -> LayoutMeasureContext {
    LayoutMeasureContext {
        measure: StackSafe::new(Box::new(move |_, _, _, _| {
            MeasuredLayoutResult::Size(size(px(width), px(height)))
        })),
        text_hydrator: None,
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ModelRootKey {
    Global {
        site: u8,
        id: u8,
    },
    Anonymous {
        site: u8,
        stack: Vec<u8>,
        occurrence: u64,
    },
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
            registry.begin_frame();
            let mut anonymous_occurrences = HashMap::<(u8, Vec<u8>), u64>::new();
            let request_count = draw_usize(&tc, 0, 10);
            let mut actual = Vec::new();
            let mut expected = Vec::new();

            for _ in 0..request_count {
                let site = draw_u8(&tc, 0, 2);
                let root_site = root_site(site);
                let stack_key = draw_element_stack(&tc);
                let stack = element_stack(&stack_key);
                let use_global = tc.draw(generators::booleans());
                let model_key = if use_global {
                    let id = draw_u8(&tc, 0, 4);
                    let global = global_id(id);
                    actual.push(registry.retained_root_id(root_site, Some(&global), &stack));
                    ModelRootKey::Global { site, id }
                } else {
                    actual.push(registry.retained_root_id(root_site, None, &stack));
                    let occurrence = anonymous_occurrences
                        .entry((site, stack_key.clone()))
                        .and_modify(|occurrence| *occurrence += 1)
                        .or_insert(0);
                    let key = ModelRootKey::Anonymous {
                        site,
                        stack: stack_key,
                        occurrence: *occurrence,
                    };
                    *occurrence += 1;
                    key
                };

                let root_id = *model.entry(model_key).or_insert_with(|| {
                    let root_id = RetainedLayoutRootId::new(next_root_id);
                    next_root_id += 1;
                    root_id
                });
                expected.push(root_id);
            }

            assert_eq!(actual, expected);
        }
    })
    .settings(hegel_settings(128))
    .run();
}

#[gpui::test]
fn frame_intents_checkpoint_restores_exact_intent_log(_cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let before_count = draw_usize(&tc, 0, 16);
        let after_count = draw_usize(&tc, 0, 16);
        let mut frame = FrameIntents::new();
        let mut expected = Vec::new();

        for _ in 0..before_count {
            let intent = unmeasured_intent(Vec::new());
            let id = frame.push_intent(intent.clone());
            expected.push((id, intent));
        }

        let checkpoint = frame.checkpoint();
        for _ in 0..after_count {
            let _ = frame.push_intent(measured_intent(MeasuredLayoutFacts::opaque()));
        }
        frame.rollback_to_checkpoint(checkpoint);

        assert_eq!(
            expected.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
            (0..before_count).map(LayoutId).collect::<Vec<_>>()
        );
        assert_eq!(
            expected
                .iter()
                .map(|(id, _)| (*id, frame.intent(*id).clone()))
                .collect::<Vec<_>>(),
            expected
        );
        let next = frame.push_intent(unmeasured_intent(Vec::new()));
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

        for index in 0..before_count {
            let node = new_test_node(&mut solver);
            state.mark_solver_node_committed(node);
            state.insert(LayoutId(index), node);
            before.push((LayoutId(index), node));
        }

        let checkpoint = state.checkpoint();
        for index in before_count..(before_count + after_count) {
            let node = new_test_node(&mut solver);
            state.mark_solver_node_committed(node);
            state.insert(LayoutId(index), node);
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
        assert_eq!(state.try_node(LayoutId(before_count)), None);
        let next_node = new_test_node(&mut solver);
        state.mark_solver_node_committed(next_node);
        state.insert(LayoutId(before_count), next_node);
        assert_eq!(state.try_node(LayoutId(before_count)), Some(next_node));
    })
    .settings(hegel_settings(128))
    .run();
}

#[gpui::test]
fn root_slots_checkpoint_restores_current_roots_and_detached_removals(_cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let mut solver = LayoutSolver::new();
        let root_a = RetainedLayoutRootId::new(draw_u8(&tc, 0, 10).into());
        let root_b = RetainedLayoutRootId::new((draw_u8(&tc, 11, 20)).into());
        let occurrence_a = occurrence(new_test_node(&mut solver));
        let detached_a = occurrence(new_test_node(&mut solver));
        let occurrence_b = occurrence(new_test_node(&mut solver));
        let detached_b = occurrence(new_test_node(&mut solver));
        let mut slots = RootSlots::new();

        slots.insert_current_root(root_a, occurrence_a.clone());
        slots.detach_subtree(detached_a.clone());
        let checkpoint = slots.checkpoint();
        slots.insert_current_root(root_b, occurrence_b);
        slots.detach_subtree(detached_b);
        slots.rollback_to_checkpoint(checkpoint);
        slots.promote_current_roots();

        assert_eq!(slots.take_retained_root(root_a), Some(occurrence_a));
        assert_eq!(slots.take_retained_root(root_b), None);
        assert_eq!(slots.take_detached_subtree_removals(), vec![detached_a]);
    })
    .settings(hegel_settings(128))
    .run();
}

#[gpui::test]
fn bounds_cache_checkpoint_restores_cached_absolute_bounds(_cx: &mut TestAppContext) {
    hegel::Hegel::new(|tc| {
        let width = draw_u8(&tc, 0, 200) as f32;
        let height = draw_u8(&tc, 0, 200) as f32;
        let scale_factor = draw_u8(&tc, 1, 4) as f32;
        let mut solver = LayoutSolver::new();
        let child = solver.new_leaf(SolverStyle::test_with_size(width, height));
        let root = solver.new_with_children(SolverStyle::default(), &[child]);
        solver.compute_layout_with_measure_and_cache_events(
            root,
            size(
                super::super::AvailableSpace::Definite(px(width.max(1.0))),
                super::super::AvailableSpace::Definite(px(height.max(1.0))),
            ),
            1.0,
            |_node_id, _has_measure_context, _query| size(0.0, 0.0),
            |_| {},
        );
        let layouts = solver
            .capture_layout_tree(root)
            .into_iter()
            .collect::<HashMap<_, _>>();

        let mut cache = BoundsCache::new();
        let first = cache.layout_bounds_for_node(
            child,
            scale_factor,
            |node_id| layouts[&node_id],
            |node_id| solver.parent(node_id),
        );
        let checkpoint = cache.checkpoint();
        let _ = cache.layout_bounds_for_node(
            root,
            scale_factor,
            |node_id| layouts[&node_id],
            |node_id| solver.parent(node_id),
        );
        cache.rollback_to_checkpoint(checkpoint);
        let second = cache.layout_bounds_for_node(
            child,
            scale_factor,
            |node_id| layouts[&node_id],
            |node_id| solver.parent(node_id),
        );

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
