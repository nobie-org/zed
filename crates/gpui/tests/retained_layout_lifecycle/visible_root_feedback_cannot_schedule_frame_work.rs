use gpui::{App, VisibleRootFeedbackCx};

fn bad(feedback: &mut VisibleRootFeedbackCx<'_>, _cx: &mut App) {
    feedback.frame_prepaint_plan(|_, _| ());
}

fn main() {}
