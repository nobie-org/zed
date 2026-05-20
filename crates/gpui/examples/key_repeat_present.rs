#![cfg_attr(target_family = "wasm", no_main)]

use gpui::{
    App, Bounds, Context, FocusHandle, InteractiveElement, Window, WindowBounds, WindowOptions,
    div, nobie_platform_trace, prelude::*, px, rgb, size,
};
use gpui_platform::application;
use std::time::Duration;

struct KeyRepeatPresent {
    focus_handle: FocusHandle,
    generation: usize,
    blue: bool,
    render_sleep_ms: u64,
}

impl KeyRepeatPresent {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            focus_handle,
            generation: 0,
            blue: false,
            render_sleep_ms: std::env::var("GPUI_KEY_REPEAT_RENDER_SLEEP_MS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        }
    }
}

impl Render for KeyRepeatPresent {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        eprintln!(
            "gpui-key-repeat-example event=render generation={} blue={} render_sleep_ms={}",
            self.generation, self.blue, self.render_sleep_ms
        );
        nobie_platform_trace::trace(
            "example_render",
            format_args!(
                "generation={} blue={} render_sleep_ms={}",
                self.generation, self.blue, self.render_sleep_ms
            ),
        );
        if self.render_sleep_ms > 0 {
            std::thread::sleep(Duration::from_millis(self.render_sleep_ms));
        }
        let focus_handle = self.focus_handle.clone();
        div()
            .id("key-repeat-present")
            .track_focus(&focus_handle)
            .on_key_down(cx.listener(|this, _, _, cx| {
                this.generation = this.generation.saturating_add(1);
                this.blue = !this.blue;
                eprintln!(
                    "gpui-key-repeat-example event=key_down generation={} blue={}",
                    this.generation, this.blue
                );
                nobie_platform_trace::trace(
                    "example_key_down",
                    format_args!("generation={} blue={}", this.generation, this.blue),
                );
                cx.notify();
            }))
            .size(px(420.0))
            .flex()
            .flex_col()
            .justify_center()
            .items_center()
            .gap_4()
            .bg(if self.blue {
                rgb(0x1d4ed8)
            } else {
                rgb(0xdc2626)
            })
            .text_color(rgb(0xffffff))
            .text_xl()
            .child("Hold ArrowDown")
            .child(format!("generation {}", self.generation))
            .child(if self.blue { "blue" } else { "red" })
    }
}

fn run_example() {
    application().run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(420.0), px(420.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| KeyRepeatPresent::new(window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}

#[cfg(not(target_family = "wasm"))]
fn main() {
    run_example();
}

#[cfg(target_family = "wasm")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    gpui_platform::web_init();
    run_example();
}
