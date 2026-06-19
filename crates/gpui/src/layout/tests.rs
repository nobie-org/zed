use super::*;
use crate::{
    AbsoluteLength, AppContext as _, DefiniteLength, ElementId, GlobalElementId, Length,
    SharedString, TextLayoutArtifact, TextMeasureKey, TextStyle, size,
};
use std::{cell::Cell, ops::Deref as _, rc::Rc, sync::Arc, time::Duration};

fn global_id(name: &str) -> GlobalElementId {
    GlobalElementId(Arc::from([ElementId::Name(name.into())]))
}

#[test]
fn layout_work_sample_counts_requests_and_finish_frame_resets() {
    let mut engine = LayoutEngine::new();
    let first_child = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[]);
    let second_child = engine.request_layout(Style::default(), Pixels(16.0), 1.0, &[]);
    engine.request_layout(
        Style::default(),
        Pixels(16.0),
        1.0,
        &[first_child, second_child],
    );

    let expected_sample = LayoutWorkSample {
        draw_index: 0,
        layout_node_requests: 3,
        measured_layout_node_requests: 0,
        child_edges: 2,
        compute_layout_calls: 0,
        measured_layout_calls: 0,
        compute_layout_duration: Duration::default(),
        measured_layout_duration: Duration::default(),
        ..LayoutWorkSample::default()
    };
    assert_eq!(engine.layout_work_sample(), expected_sample);
    assert_eq!(engine.finish_frame(), expected_sample);
    assert_eq!(engine.layout_work_sample(), LayoutWorkSample::default());
}

#[test]
fn layout_work_sample_counts_compute_and_measure() {
    let measure_invocations = Rc::new(Cell::new(0));
    let measure_invocations_for_closure = measure_invocations.clone();
    let mut test_app = crate::TestAppContext::single();
    let window = test_app.add_window(|_, _| crate::Empty);

    let sample = test_app
        .update_window(*window.deref(), |_, window, cx| {
            let mut engine = LayoutEngine::new();
            let measured_layout = engine.request_measured_layout(
                Style::default(),
                Pixels(16.0),
                1.0,
                move |_, _, _, _| {
                    measure_invocations_for_closure.set(measure_invocations_for_closure.get() + 1);
                    std::thread::sleep(Duration::from_micros(1));
                    size(Pixels(10.0), Pixels(20.0))
                },
            );

            engine.compute_layout(
                measured_layout,
                size(
                    AvailableSpace::Definite(Pixels(100.0)),
                    AvailableSpace::Definite(Pixels(100.0)),
                ),
                window,
                cx,
            );
            engine.finish_frame()
        })
        .unwrap();

    assert_eq!(
        sample,
        LayoutWorkSample {
            draw_index: 0,
            layout_node_requests: 0,
            measured_layout_node_requests: 1,
            child_edges: 0,
            compute_layout_calls: 1,
            solver_compute_layout_calls: 1,
            measured_layout_calls: 1,
            compute_layout_duration: sample.compute_layout_duration,
            measured_layout_duration: sample.measured_layout_duration,
            retained_layout_creates: 1,
            retained_layout_miss_no_previous: 1,
            ..LayoutWorkSample::default()
        }
    );
    assert_eq!(measure_invocations.get(), 1);
    assert!(sample.compute_layout_duration >= sample.measured_layout_duration);
    assert!(sample.measured_layout_duration > Duration::default());
}

#[test]
fn layout_work_sample_reports_retained_layout_miss_reasons() {
    let request_leaf = |engine: &mut LayoutEngine, width| {
        let mut style = Style::default();
        style.size.width = Length::Definite(DefiniteLength::Absolute(AbsoluteLength::Pixels(
            Pixels(width),
        )));
        engine.request_layout(style, Pixels(16.0), 1.0, &[])
    };

    let mut engine = LayoutEngine::new();
    let root = request_leaf(&mut engine, 10.0);
    engine.commit_layout(root);
    engine.finish_frame();

    let root = request_leaf(&mut engine, 20.0);
    engine.commit_layout(root);

    assert_eq!(
        engine.finish_frame(),
        LayoutWorkSample {
            layout_node_requests: 1,
            retained_layout_reuses: 1,
            retained_layout_style_updates: 1,
            ..LayoutWorkSample::default()
        }
    );
}

#[test]
fn stable_subtree_probe_reports_zero_write_work_after_admission() {
    let mut test_app = crate::TestAppContext::single();
    let window = test_app.add_window(|_, _| crate::Empty);
    let global_id = global_id("tracked-subtree");
    let mut engine = LayoutEngine::new();
    engine.set_retained_subtree_probe_targets_for_tests(vec!["tracked-subtree".to_string()]);

    let root = engine.request_layout_with_global_id(
        Some(&global_id),
        Style::default(),
        Pixels(16.0),
        1.0,
        &[],
    );
    test_app
        .update_window(*window.deref(), |_, window, cx| {
            engine.compute_layout(
                root,
                size(
                    AvailableSpace::Definite(Pixels(100.0)),
                    AvailableSpace::Definite(Pixels(100.0)),
                ),
                window,
                cx,
            );
        })
        .unwrap();
    engine.finish_frame();

    let root = engine.request_layout_with_global_id(
        Some(&global_id),
        Style::default(),
        Pixels(16.0),
        1.0,
        &[],
    );
    test_app
        .update_window(*window.deref(), |_, window, cx| {
            engine.compute_layout(
                root,
                size(
                    AvailableSpace::Definite(Pixels(100.0)),
                    AvailableSpace::Definite(Pixels(100.0)),
                ),
                window,
                cx,
            );
        })
        .unwrap();

    let samples = engine.retained_subtree_work_samples_for_tests();
    let expected_sample = RetainedSubtreeWorkSample {
        global_id: "tracked-subtree".to_string(),
        layout_id: 0,
        node_count: 1,
        retained_reuses: 1,
        ..RetainedSubtreeWorkSample::default()
    };
    assert_eq!(samples, std::slice::from_ref(&expected_sample));
    assert_eq!(samples[0].no_work_total(), 0);

    engine.finish_frame();
    assert_eq!(
        engine.last_retained_subtree_work_samples_for_tests(),
        std::slice::from_ref(&expected_sample)
    );
    assert_eq!(engine.retained_subtree_work_samples_for_tests(), &[]);
}

#[test]
fn subtree_probe_attributes_measured_callbacks_to_tagged_parent() {
    let mut test_app = crate::TestAppContext::single();
    let window = test_app.add_window(|_, _| crate::Empty);
    let global_id = global_id("tracked-measured-parent");
    let mut engine = LayoutEngine::new();
    engine
        .set_retained_subtree_probe_targets_for_tests(vec!["tracked-measured-parent".to_string()]);

    let measured = engine.request_pure_measured_layout(
        Style::default(),
        Pixels(16.0),
        1.0,
        PureSizeMeasure::content_size(size(Pixels(10.0), Pixels(20.0)), 1.0),
    );
    let root = engine.request_layout_with_global_id(
        Some(&global_id),
        Style::default(),
        Pixels(16.0),
        1.0,
        &[measured],
    );

    test_app
        .update_window(*window.deref(), |_, window, cx| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                cx,
            );
        })
        .unwrap();

    let samples = engine.retained_subtree_work_samples_for_tests();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].global_id, "tracked-measured-parent");
    assert_eq!(samples[0].node_count, 2);
    assert_eq!(samples[0].measured_callbacks, 1);
    assert!(samples[0].no_work_total() > 0);
}

#[test]
fn subtree_probe_reports_conservative_text_callbacks_separately() {
    let mut test_app = crate::TestAppContext::single();
    let window = test_app.add_window(|_, _| crate::Empty);
    let global_id = global_id("tracked-text-parent");
    let mut engine = LayoutEngine::new();
    engine.set_retained_subtree_probe_targets_for_tests(vec!["tracked-text-parent".to_string()]);

    let text = SharedString::new_static("tracked text");
    let text_style = TextStyle::default();
    let key = TextMeasureKey::new(
        text.clone(),
        vec![text_style.to_run(text.len())],
        &text_style,
        Pixels(16.0),
        Pixels(20.0),
        1.0,
        0,
    );
    let artifact = TextLayoutArtifact::for_tests(key.clone(), size(Pixels(20.0), Pixels(20.0)));
    let text_layout = engine.request_text_measured_layout(
        Style::default(),
        Pixels(16.0),
        1.0,
        key,
        |_| {},
        move |_, _, _, _| artifact.clone(),
    );
    let root = engine.request_layout_with_global_id(
        Some(&global_id),
        Style::default(),
        Pixels(16.0),
        1.0,
        &[text_layout],
    );

    test_app
        .update_window(*window.deref(), |_, window, cx| {
            engine.compute_layout(
                root,
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                cx,
            );
        })
        .unwrap();

    let samples = engine.retained_subtree_work_samples_for_tests();
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].global_id, "tracked-text-parent");
    assert_eq!(samples[0].node_count, 2);
    assert_eq!(samples[0].measured_callbacks, 1);
    assert_eq!(samples[0].conservative_text_measured_callbacks, 1);
    assert_eq!(
        samples[0].no_work_total(),
        samples[0].retained_misses
            + samples[0].mirror_node_creates
            + samples[0].mirror_node_removes
            + samples[0].mirror_set_style
            + samples[0].mirror_set_children
            + samples[0].mirror_measured_context_clears
    );
}
