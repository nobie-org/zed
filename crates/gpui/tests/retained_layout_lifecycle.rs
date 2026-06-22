#[test]
fn retained_layout_lifecycle_misuse_is_unrepresentable() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/retained_layout_lifecycle/*.rs");
}

#[test]
fn deleted_generic_frame_work_surface_does_not_exist() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let window = std::fs::read_to_string(crate_root.join("src/window.rs")).unwrap();

    for forbidden in [
        "trait FramePrepaintWork",
        "fn schedule_frame_prepaint_work",
        "fn frame_prepaint_plan",
        "frame_prepaint_plans",
        "FramePrepaintIntent",
        "FramePrepaintRun",
        "struct FrameValue",
        "struct FrameValueWriter",
        "fn take_frame_value",
        "fn paint_owner_painted_visible_root",
        "SolvedVisibleRoot",
        "fn solve_visible_root",
        "fn prepaint_solved_visible_root_at",
        "fn defer_solved_visible_root_at",
        "draw_and_present_immediately",
        "draw_present_and_capture_immediately",
    ] {
        assert!(
            !window.contains(forbidden),
            "deleted retained-lifecycle escape hatch must not reappear: {forbidden}"
        );
    }
}

#[test]
fn intermediate_frame_layout_vocabulary_does_not_exist_in_production_sources() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
    let mut offenders = Vec::new();

    for path in rust_source_files(&source_root) {
        let relative = path.strip_prefix(&source_root).unwrap();
        let relative = relative.to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();

        for forbidden in [
            "VisibleRootPlan",
            "visible_root_plan",
            "FramePrepaintCx",
            "ListVisibleRootToken",
            "ListFramePrepaintCx",
            "schedule_frame_prepaint_work",
            "schedule_list_prepaint_work",
            "register_list_prepaint",
            "list_prepaints",
            "PendingListPrepaint",
            "ListPrepaintIntent",
            "PrepaintItemsResponse",
            "layout_visible_root_size",
            "prepaint_list_visible_root_at",
        ] {
            if source.contains(forbidden) {
                offenders.push(format!("{relative}: {forbidden}"));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "intermediate retained-layout scheduling vocabulary must not survive: {offenders:?}"
    );
}

#[test]
fn tooltip_lifecycle_authority_is_opaque() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let app = std::fs::read_to_string(crate_root.join("src/app.rs")).unwrap();
    let tooltip_start = app.find("pub struct AnyTooltip").unwrap();
    let tooltip_end = app[tooltip_start..]
        .find("/// A keystroke event")
        .map(|offset| tooltip_start + offset)
        .unwrap();
    let tooltip_source = &app[tooltip_start..tooltip_end];

    for forbidden in [
        "pub view: AnyView",
        "pub mouse_position: Point<Pixels>",
        "pub check_visible_and_update",
        "Fn(Bounds<Pixels>, &mut Window, &mut App)",
        "fn window(&self) -> &Window",
        "fn window_mut(&mut self) -> &mut Window",
        "pub(crate) fn window",
        "pub(crate) fn window_mut",
    ] {
        assert!(
            !tooltip_source.contains(forbidden),
            "tooltip state must not expose raw window/layout authority: {forbidden}"
        );
    }
}

#[test]
fn div_observation_hooks_do_not_expose_prepaint_authority() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let div = std::fs::read_to_string(crate_root.join("src/elements/div.rs")).unwrap();

    for forbidden in [
        "Fn(Vec<Bounds<Pixels>>, &mut PrepaintCx<'_>, &mut App)",
        "impl Fn(Vec<Bounds<Pixels>>, &mut PrepaintCx<'_>, &mut App)",
        "Fn(&mut PrepaintCx<'_>, &mut App) -> SmallVec",
        "impl Fn(&mut PrepaintCx<'_>, &mut App) -> SmallVec",
    ] {
        assert!(
            !div.contains(forbidden),
            "div observation/order hooks must not expose root scheduling authority: {forbidden}"
        );
    }
}

#[test]
fn scratch_measurement_is_not_public_prepaint_authority() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let window = std::fs::read_to_string(crate_root.join("src/window.rs")).unwrap();

    for forbidden in [
        "measure_size_only_root(",
        "measure_scratch_root_size(",
        "measure_detached_root_size(",
        "measure_scratch_root(",
    ] {
        assert!(
            !window.contains(forbidden),
            "scratch layout solves must not be public prepaint authority: {forbidden}"
        );
    }
}

#[test]
fn grouped_visible_roots_are_registered_intents_not_solve_now_helpers() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let window = std::fs::read_to_string(crate_root.join("src/window.rs")).unwrap();

    assert!(
        !window.contains("Vec<Option<DeferredVisibleRootPlacement>>"),
        "grouped visible roots must not support solve-then-omit placement"
    );
    assert!(
        !window.contains("let Some(placement) = placement"),
        "grouped visible-root drain must not solve a retained root and then skip it"
    );
    for forbidden in [
        "pub fn new(output: T, placements: Vec<DeferredVisibleRootPlacement>)",
        "enum VisibleRootVisibility",
        "VisibleRootVisibility::",
        "VisibleRootState::Cancelled",
    ] {
        assert!(
            !window.contains(forbidden),
            "visible-root lifecycle must not support solve-then-cancel or solve-then-hide states: {forbidden}"
        );
    }

    let prepaint_method = source_item(&window, "pub fn defer_visible_root_group");
    assert!(
        prepaint_method.contains("register_visible_root_group"),
        "PrepaintCx::defer_visible_root_group must register a grouped visible-root intent"
    );
    for forbidden in [
        "layout_detached_root_size",
        "layout_visible_root_size",
        "prepaint_at",
        "defer_draw",
    ] {
        assert!(
            !prepaint_method.contains(forbidden),
            "defer_visible_root_group must not solve, prepaint, or defer raw draw work synchronously: {forbidden}"
        );
    }

    let register_method = source_item(&window, "fn register_visible_root_group");
    assert!(
        register_method.contains("visible_root_groups"),
        "register_visible_root_group must enqueue grouped visible-root state"
    );
    assert!(
        !register_method.contains("layout_detached_root_size"),
        "register_visible_root_group must not execute a solve while registering the intent"
    );
}

#[test]
fn visible_root_group_context_does_not_carry_raw_layout_frame() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let window = std::fs::read_to_string(crate_root.join("src/window.rs")).unwrap();
    let group_cx = source_item(&window, "pub struct VisibleRootGroupCx");

    for forbidden in ["LayoutFrame", "layout_frame"] {
        assert!(
            !group_cx.contains(forbidden),
            "visible-root group placement must not carry raw frame solve authority: {forbidden}"
        );
    }
}

#[test]
fn layout_frame_size_and_visible_root_methods_are_not_crate_authority() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let window = std::fs::read_to_string(crate_root.join("src/window.rs")).unwrap();
    let layout_frame = source_item(&window, "impl LayoutFrame");

    for forbidden in [
        "fn prepaint_window_root_at(",
        "fn prepaint_detached_root_at(",
        "fn prepaint_detached_root_at_with_identity(",
        "pub(crate) fn layout_visible_root(",
        "pub(crate) fn measure_size_only_root(",
        "pub(crate) fn compute_detached_root_layout(",
        "pub(crate) fn layout_bounds(&self, window: &Window, layout_id: LayoutId)",
    ] {
        assert!(
            !layout_frame.contains(forbidden),
            "LayoutFrame must not expose direct root sizing authority to arbitrary GPUI modules: {forbidden}"
        );
    }

    for forbidden in [
        "pub(crate) fn with_test_layout_frame(",
        "pub(crate) fn layout_drawable_as_root_size(",
        "fn layout_drawable_as_root_size(",
    ] {
        assert!(
            !window.contains(forbidden),
            "test support must not expose raw frame solve authority: {forbidden}"
        );
    }
}

#[test]
fn raw_window_layout_authority_is_module_private() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let window = std::fs::read_to_string(crate_root.join("src/window.rs")).unwrap();
    let raw_window_impl_start = window
        .find("impl Window {\n    fn mark_view_dirty")
        .expect("raw Window impl should exist");
    let raw_window_impl_end = window[raw_window_impl_start..]
        .find("\n// #[derive(Clone, Copy, Eq, PartialEq, Hash)]")
        .map(|offset| raw_window_impl_start + offset)
        .expect("raw Window impl should end before WindowId");
    let raw_window_impl = &window[raw_window_impl_start..raw_window_impl_end];

    for forbidden in [
        "pub fn request_layout(\n        &mut self,\n        style: Style,",
        "pub(crate) fn request_layout(\n        &mut self,\n        style: Style,",
        "pub(crate) fn request_layout_with_global_id(",
        "pub(crate) fn request_measured_layout<F>(&mut self, style: Style, measure: F)",
        "pub(crate) fn request_pure_measured_layout(",
        "pub(crate) fn request_content_size_measured_layout(",
        "pub(crate) fn request_text_measured_layout<F, H>(",
        "pub(crate) fn layout_bounds(&self, layout_id: LayoutId)",
        "pub fn draw(&mut self, cx: &mut App)",
        "pub(crate) fn draw(&mut self, cx: &mut App)",
        "pub fn draw_and_present_immediately(&mut self, cx: &mut App)",
        "pub fn draw_present_and_capture_immediately(&mut self, cx: &mut App)",
    ] {
        assert!(
            !raw_window_impl.contains(forbidden),
            "raw Window must not expose layout authority outside window.rs: {forbidden}"
        );
    }
}

#[test]
fn window_draw_requires_app_frame_authority() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
    let window = std::fs::read_to_string(source_root.join("window.rs")).unwrap();
    let draw_for_app = source_item(&window, "pub(crate) fn draw_for_app");

    assert!(
        draw_for_app.contains("WindowFrameAuthority"),
        "app-visible draw must require the app-owned frame authority"
    );

    let mut offenders = Vec::new();
    for path in rust_source_files(&source_root) {
        let relative = path.strip_prefix(&source_root).unwrap();
        let relative = relative.to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();

        if !source.contains("draw_for_app(") {
            continue;
        }

        let allowed = relative == "window.rs"
            || relative == "app.rs"
            || relative == "app/headless_app_context.rs"
            || relative == "app/test_app.rs"
            || relative == "app/test_context.rs";
        if !allowed {
            offenders.push(relative.into_owned());
        }
    }

    assert!(
        offenders.is_empty(),
        "only the app runtime and test harness may initiate app-authorized window draw: {offenders:?}"
    );
}

#[test]
fn list_frame_root_authority_does_not_have_a_special_context() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
    let mut offenders = Vec::new();

    for path in rust_source_files(&source_root) {
        let relative = path.strip_prefix(&source_root).unwrap();
        let relative = relative.to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();

        if !source.contains("ListFramePrepaintCx")
            && !source.contains("ListVisibleRootToken")
            && !source.contains("schedule_list_prepaint_work")
            && !source.contains("prepaint_list_visible_root_at")
        {
            continue;
        }
        offenders.push(relative.into_owned());
    }

    assert!(
        offenders.is_empty(),
        "list root-layout authority must use the same retained-frame primitives as other roots, found special contexts in {offenders:?}"
    );
}

#[test]
fn list_custom_layout_does_not_receive_raw_layout_frame() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let list = std::fs::read_to_string(crate_root.join("src/elements/list.rs")).unwrap();

    for forbidden in [
        "CustomLayoutCx",
        "LayoutFrame",
        "layout_frame",
        "LaidOutVisibleRoot",
        "PrepaintedVisibleRoot",
    ] {
        assert!(
            !list.contains(forbidden),
            "list custom layout must use inert custom-layout steps, not raw frame/root authority: {forbidden}"
        );
    }
    assert!(
        list.contains("CustomLayoutStep"),
        "list should express variable-height work through the generic custom-layout step protocol"
    );
}

#[test]
fn custom_layout_step_protocol_is_inert_and_frame_drained() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
    let window = std::fs::read_to_string(source_root.join("window.rs")).unwrap();
    let custom_layout_step = source_item(&window, "pub(crate) enum CustomLayoutStep<T>");

    for required in [
        "BuildVisibleRoot",
        "PrepaintVisibleRoot",
        "TakeAutoscroll",
        "ContainsFocused",
        "RestartAttempt",
        "Finish(T)",
    ] {
        assert!(
            custom_layout_step.contains(required),
            "custom layout step protocol should contain the final inert operation: {required}"
        );
    }

    for forbidden in [
        "pub fn ",
        "&mut Window",
        "LayoutFrame",
        "fn window(",
        "fn window_mut(",
        "request_layout(",
        "compute_retained_layout(",
        "prepaint_detached_root_at(",
        "measure_size_only_root(",
        "layout_detached_root_size(",
        "draw(",
        "defer_visible_root(",
        "defer_visible_root_group(",
        "insert_hitbox(",
        "on_mouse_event(",
    ] {
        assert!(
            !custom_layout_step.contains(forbidden),
            "custom layout step protocol must not expose raw window/frame/prepaint authority: {forbidden}"
        );
    }

    let mut offenders = Vec::new();
    for path in rust_source_files(&source_root) {
        let relative = path.strip_prefix(&source_root).unwrap();
        let relative = relative.to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();

        if !source.contains("CustomLayoutCx") {
            continue;
        }

        offenders.push(relative.into_owned());
    }

    assert!(
        offenders.is_empty(),
        "CustomLayoutCx must not exist; custom layout is inert step data drained by the private frame owner, found in {offenders:?}"
    );
}

#[test]
fn laid_out_visible_root_is_a_consumed_value_not_loose_geometry() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let window = std::fs::read_to_string(crate_root.join("src/window.rs")).unwrap();
    let laid_out_root = source_item(&window, "pub(crate) struct LaidOutVisibleRoot");
    let laid_out_impl = source_item(&window, "impl LaidOutVisibleRoot");

    for forbidden in [
        "pub element",
        "pub size",
        "pub(crate) element",
        "pub(crate) size",
    ] {
        assert!(
            !laid_out_root.contains(forbidden),
            "laid-out visible root must not expose loose geometry or element storage: {forbidden}"
        );
    }

    assert!(
        laid_out_impl.contains("fn prepaint_at(\n        mut self,"),
        "laid-out visible roots must be consumed by prepaint so callers cannot solve one root and paint another"
    );
}

#[test]
fn retained_solver_execution_is_private_to_layout_frame() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
    let mut offenders = Vec::new();

    for path in rust_source_files(&source_root) {
        let relative = path.strip_prefix(&source_root).unwrap();
        let relative = relative.to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();

        if !source.contains("compute_retained_layout(") {
            continue;
        }

        if relative != "window.rs" && relative != "layout.rs" {
            offenders.push(relative.into_owned());
        }
    }

    assert!(
        offenders.is_empty(),
        "retained solve execution must stay private to LayoutFrame/LayoutEngine, found in {offenders:?}"
    );
}

#[test]
fn draw_test_element_uses_layout_frame_not_manual_root_solves() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let window = std::fs::read_to_string(crate_root.join("src/window.rs")).unwrap();
    let draw_test_element = source_item(&window, "pub(crate) fn draw_test_element");

    for forbidden in [
        "DetachedRootLayoutPass",
        "request_detached_root_layout(",
        "compute_detached_root_layout(",
        "mark_detached_root_layout_computed(",
    ] {
        assert!(
            !draw_test_element.contains(forbidden),
            "test drawing must use LayoutFrame's root-layout path, not manual detached-root solve steps: {forbidden}"
        );
    }

    assert!(
        draw_test_element.contains("layout_frame.layout_detached_root_size("),
        "test drawing should route typed drawables through the private LayoutFrame root-layout path"
    );
}

#[test]
fn raw_taffy_is_hidden_behind_retained_solver_backend() {
    let crate_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
    let mut offenders = Vec::new();

    for path in rust_source_files(&source_root) {
        let relative = path.strip_prefix(&source_root).unwrap();
        let relative = relative.to_string_lossy();
        let source = std::fs::read_to_string(&path).unwrap();

        if !source.contains("taffy::") && !source.contains("TaffyTree") {
            continue;
        }

        let allowed = relative == "layout/retained_forest/solver/backend.rs"
            || relative == "layout/retained_layout_tests.rs"
            || relative == "style.rs";
        if !allowed {
            offenders.push(relative.into_owned());
        }
    }

    assert!(
        offenders.is_empty(),
        "raw Taffy details must stay behind the solver backend, found in {offenders:?}"
    );
}

fn rust_source_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];

    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    files
}

fn source_item<'a>(source: &'a str, needle: &str) -> &'a str {
    let start = source
        .find(needle)
        .unwrap_or_else(|| panic!("source item not found: {needle}"));
    let mut depth = 0usize;
    let mut saw_body = false;
    for (offset, byte) in source[start..].bytes().enumerate() {
        match byte {
            b'{' => {
                saw_body = true;
                depth += 1;
            }
            b'}' if saw_body => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..start + offset + 1];
                }
            }
            _ => {}
        }
    }
    panic!("source item body was not closed: {needle}");
}
