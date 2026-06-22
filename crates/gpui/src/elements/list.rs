//! A list element that can be used to render a large number of differently sized elements
//! efficiently. Clients of this API need to ensure that elements outside of the scrolled
//! area do not change their height for this element to function correctly. If your elements
//! do change height, notify the list element via [`ListState::splice`] or [`ListState::reset`].
//! In order to minimize re-renders, this element's state is stored intrusively
//! on your own views, so that your code can coordinate directly with the list element's cached state.
//!
//! If all of your elements are the same height, see [`crate::UniformList`] for a simpler API

use crate::{
    AnyElement, App, AvailableSpace, Bounds, BuildCx, ContentMask, CustomLayoutPaintRoot,
    CustomLayoutRoot, CustomLayoutStep, DispatchPhase, Edges, Element, EntityId, FocusHandle,
    FramePrepaintOutput, GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement,
    LayoutRequestCx, Overflow, PaintCx, Pixels, Point, PrepaintCx, ScrollDelta, ScrollWheelEvent,
    Size, Style, StyleRefinement, Styled, Window, layout::PureSizeMeasure, point, px, size,
};
use collections::VecDeque;
use refineable::Refineable as _;
use std::{cell::RefCell, ops::Range, rc::Rc};
use sum_tree::{Bias, Dimensions, SumTree};

type RenderItemFn = dyn FnMut(usize, &mut BuildCx<'_>, &mut App) -> AnyElement + 'static;
type RenderItemHandle = Rc<RefCell<Box<RenderItemFn>>>;

/// Construct a new list element
pub fn list(
    state: ListState,
    render_item: impl FnMut(usize, &mut BuildCx<'_>, &mut App) -> AnyElement + 'static,
) -> List {
    List {
        state,
        render_item: Rc::new(RefCell::new(Box::new(render_item))),
        style: StyleRefinement::default(),
        sizing_behavior: ListSizingBehavior::default(),
    }
}

/// A list element
pub struct List {
    state: ListState,
    render_item: RenderItemHandle,
    style: StyleRefinement,
    sizing_behavior: ListSizingBehavior,
}

impl List {
    /// Set the sizing behavior for the list.
    pub fn with_sizing_behavior(mut self, behavior: ListSizingBehavior) -> Self {
        self.sizing_behavior = behavior;
        self
    }
}

/// The list state that views must hold on behalf of the list element.
#[derive(Clone)]
pub struct ListState(Rc<RefCell<StateInner>>);

impl std::fmt::Debug for ListState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ListState")
    }
}

struct StateInner {
    last_layout_bounds: Option<Bounds<Pixels>>,
    last_padding: Option<Edges<Pixels>>,
    items: SumTree<ListItem>,
    logical_scroll_top: Option<ListOffset>,
    alignment: ListAlignment,
    overdraw: Pixels,
    reset: bool,
    #[allow(clippy::type_complexity)]
    scroll_handler: Option<Box<dyn FnMut(&ListScrollEvent, &mut Window, &mut App)>>,
    scrollbar_drag_start_height: Option<Pixels>,
    pending_scroll: Option<PendingScroll>,
    follow_state: FollowState,
}

/// Deferred scroll adjustment applied after the scroll-top item has been remeasured.
///
/// An absolute pending scroll preserves the same pixel offset into the item, which keeps
/// visible text stable while content is appended to or removed from that item. A
/// proportional pending scroll preserves the same fractional position within the item,
/// which is useful when the whole list is being resized and each item scales similarly.
#[derive(Clone)]
enum PendingScroll {
    /// Preserve the same pixel offset into the item after it is remeasured.
    Absolute { item_ix: usize, offset: Pixels },
    /// Preserve the same fractional offset into the item after it is remeasured.
    Proportional(PendingScrollFraction),
}

/// Keeps track of a fractional scroll position within an item for restoration
/// after remeasurement.
#[derive(Clone)]
struct PendingScrollFraction {
    /// The index of the item to scroll within.
    item_ix: usize,
    /// Fractional offset (0.0 to 1.0) within the item's height.
    fraction: f32,
}

/// Determines how remeasurement preserves the scroll position when the scroll-top item
/// changes height.
enum ScrollAnchor {
    /// Preserve the same pixel offset into the scroll-top item.
    Absolute,
    /// Preserve the same fractional position within the scroll-top item.
    Proportional,
}

/// Controls whether the list automatically follows new content at the end.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FollowMode {
    /// Normal scrolling — no automatic following.
    #[default]
    Normal,
    /// The list should auto-scroll along with the tail, when scrolled to bottom.
    Tail,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum FollowState {
    #[default]
    Normal,
    Tail {
        is_following: bool,
    },
}

impl FollowState {
    fn is_following(&self) -> bool {
        matches!(self, FollowState::Tail { is_following: true })
    }

    fn has_stopped_following(&self) -> bool {
        matches!(
            self,
            FollowState::Tail {
                is_following: false
            }
        )
    }

    fn start_following(&mut self) {
        if let FollowState::Tail {
            is_following: false,
        } = self
        {
            *self = FollowState::Tail { is_following: true };
        }
    }

    fn stop_following(&mut self) {
        if let FollowState::Tail { is_following: true } = self {
            *self = FollowState::Tail {
                is_following: false,
            };
        }
    }
}

/// Whether the list is scrolling from top to bottom or bottom to top.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListAlignment {
    /// The list is scrolling from top to bottom, like most lists.
    Top,
    /// The list is scrolling from bottom to top, like a chat log.
    Bottom,
}

/// A scroll event that has been converted to be in terms of the list's items.
pub struct ListScrollEvent {
    /// The range of items currently visible in the list, after applying the scroll event.
    pub visible_range: Range<usize>,

    /// The number of items that are currently visible in the list, after applying the scroll event.
    pub count: usize,

    /// Whether the list has been scrolled.
    pub is_scrolled: bool,

    /// Whether the list is currently in follow-tail mode (auto-scrolling to end).
    pub is_following_tail: bool,
}

/// The sizing behavior to apply during layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListSizingBehavior {
    /// The list should calculate its size based on the size of its items.
    Infer,
    /// The list should not calculate a fixed size.
    #[default]
    Auto,
}

/// The horizontal sizing behavior to apply during layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ListHorizontalSizingBehavior {
    /// List items' width can never exceed the width of the list.
    #[default]
    FitList,
    /// List items' width may go over the width of the list, if any item is wider.
    Unconstrained,
}

pub(crate) struct ListCustomLayoutOutput {
    scroll_top: ListOffset,
    item_layouts: VecDeque<PrepaintedItem>,
}

struct LaidOutItem {
    index: usize,
    root: CustomLayoutRoot,
}

struct PrepaintedItem {
    root: CustomLayoutPaintRoot,
}

/// Frame state used by the [List] element after layout.
pub struct ListPrepaintState {
    hitbox: Hitbox,
    layout: FramePrepaintOutput<ListCustomLayoutOutput>,
}

/// Frame-owned list work registered during ordinary prepaint.
///
/// The list element owns virtualization state and scroll policy. The frame
/// owner later runs this work with private layout/prepaint authority, which
/// keeps variable-height item solving out of `PrepaintCx`.
struct ListCustomLayoutJob {
    state: ListState,
    bounds: Bounds<Pixels>,
    padding: Edges<Pixels>,
    render_item: RenderItemHandle,
}

impl ListCustomLayoutJob {
    fn new(
        state: ListState,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        render_item: RenderItemHandle,
    ) -> Self {
        Self {
            state,
            bounds,
            padding,
            render_item,
        }
    }

    pub(crate) fn run(self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        ListLayoutAttempt {
            state: self.state,
            bounds: self.bounds,
            padding: self.padding,
            render_item: self.render_item,
            autoscroll: true,
        }
        .start()
    }
}

struct ListLayoutAttempt {
    state: ListState,
    bounds: Bounds<Pixels>,
    padding: Edges<Pixels>,
    render_item: RenderItemHandle,
    autoscroll: bool,
}

impl ListLayoutAttempt {
    fn start(self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        let (old_items, scroll_top, overdraw, alignment) = {
            let state = &mut *self.state.0.borrow_mut();
            let mut scroll_top = state.logical_scroll_top();
            if state.follow_state.is_following() {
                scroll_top = ListOffset {
                    item_ix: state.items.summary().count,
                    offset_in_item: px(0.),
                };
                state.logical_scroll_top = Some(scroll_top);
            }

            (
                state.items.clone(),
                scroll_top,
                state.overdraw,
                state.alignment,
            )
        };

        let available_item_space = size(
            AvailableSpace::Definite(self.bounds.size.width),
            AvailableSpace::MinContent,
        );
        let first_measured_index = scroll_top.item_ix;
        let pass = ListLayoutPass {
            attempt: self,
            old_items,
            measured_items: VecDeque::new(),
            item_layouts: VecDeque::new(),
            rendered_height: scroll_top.offset_in_item.min(Pixels::ZERO) + px(0.),
            scroll_top,
            first_measured_index,
            trailing_index: first_measured_index,
            previous_index: first_measured_index,
            leading_overdraw: Pixels::ZERO,
            rendered_focused_item: false,
            pending_scroll_checked: false,
            reanchor_after_upward_fill: false,
            available_item_space,
            overdraw,
            alignment,
        };

        pass.with_initial_rendered_height().layout_trailing_items()
    }
}

struct ListLayoutPass {
    attempt: ListLayoutAttempt,
    old_items: SumTree<ListItem>,
    measured_items: VecDeque<ListItem>,
    item_layouts: VecDeque<LaidOutItem>,
    rendered_height: Pixels,
    scroll_top: ListOffset,
    first_measured_index: usize,
    trailing_index: usize,
    previous_index: usize,
    leading_overdraw: Pixels,
    rendered_focused_item: bool,
    pending_scroll_checked: bool,
    reanchor_after_upward_fill: bool,
    available_item_space: Size<AvailableSpace>,
    overdraw: Pixels,
    alignment: ListAlignment,
}

impl ListLayoutPass {
    fn with_initial_rendered_height(mut self) -> Self {
        self.rendered_height = self.attempt.padding.top;
        self
    }

    fn item_at(items: &SumTree<ListItem>, index: usize) -> Option<ListItem> {
        let mut cursor = items.cursor::<Count>(());
        cursor.seek(&Count(index), Bias::Right);
        if cursor.start().0 == index {
            cursor.item().cloned()
        } else {
            None
        }
    }

    fn item_start(items: &SumTree<ListItem>, index: usize) -> Pixels {
        let mut cursor = items.cursor::<ListItemSummary>(());
        cursor.seek(&Count(index), Bias::Right);
        cursor.start().height
    }

    fn build_item(
        self,
        item_index: usize,
        item: ListItem,
        then: impl FnOnce(Self, ListItem, CustomLayoutRoot) -> CustomLayoutStep<ListCustomLayoutOutput>
        + 'static,
    ) -> CustomLayoutStep<ListCustomLayoutOutput> {
        let render_item = self.attempt.render_item.clone();
        CustomLayoutStep::BuildVisibleRoot {
            build: Box::new(move |window, cx| {
                let mut render_item = render_item.borrow_mut();
                render_item(item_index, window, cx)
            }),
            available_space: self.available_item_space,
            then: Box::new(move |root| then(self, item, root)),
        }
    }

    fn check_focus(
        mut self,
        focus_handle: Option<FocusHandle>,
        then: impl FnOnce(Self) -> CustomLayoutStep<ListCustomLayoutOutput> + 'static,
    ) -> CustomLayoutStep<ListCustomLayoutOutput> {
        if self.rendered_focused_item {
            return then(self);
        }

        let Some(focus_handle) = focus_handle else {
            return then(self);
        };

        CustomLayoutStep::ContainsFocused {
            focus_handle,
            then: Box::new(move |contains| {
                if contains {
                    self.rendered_focused_item = true;
                }
                then(self)
            }),
        }
    }

    fn apply_pending_scroll(&mut self, item_index: usize, item_size: Size<Pixels>) {
        if self.pending_scroll_checked {
            return;
        }
        self.pending_scroll_checked = true;

        let state = &mut *self.attempt.state.0.borrow_mut();
        let Some(pending_scroll) = state.pending_scroll.take() else {
            return;
        };

        match pending_scroll {
            PendingScroll::Absolute { item_ix, offset } if item_ix == item_index => {
                self.scroll_top.offset_in_item = offset.min(item_size.height);
                state.logical_scroll_top = Some(self.scroll_top);
            }
            PendingScroll::Proportional(pending_scroll) if pending_scroll.item_ix == item_index => {
                self.scroll_top.offset_in_item =
                    Pixels(pending_scroll.fraction * item_size.height.0);
                state.logical_scroll_top = Some(self.scroll_top);
            }
            _ => {}
        }
    }

    fn push_measured_back(
        &mut self,
        item_index: usize,
        item: &ListItem,
        root: CustomLayoutRoot,
    ) -> Option<FocusHandle> {
        let size = root.size();
        let focus_handle = item.focus_handle();
        self.item_layouts.push_back(LaidOutItem {
            index: item_index,
            root,
        });
        self.rendered_height += size.height;
        self.measured_items.push_back(ListItem::Measured {
            size,
            focus_handle: focus_handle.clone(),
        });
        focus_handle
    }

    fn push_measured_size_back(
        &mut self,
        item: &ListItem,
        size: Size<Pixels>,
    ) -> Option<FocusHandle> {
        let focus_handle = item.focus_handle();
        self.rendered_height += size.height;
        self.measured_items.push_back(ListItem::Measured {
            size,
            focus_handle: focus_handle.clone(),
        });
        focus_handle
    }

    fn push_measured_front(
        &mut self,
        item_index: usize,
        item: &ListItem,
        root: CustomLayoutRoot,
    ) -> Option<FocusHandle> {
        let size = root.size();
        let focus_handle = item.focus_handle();
        self.item_layouts.push_front(LaidOutItem {
            index: item_index,
            root,
        });
        self.first_measured_index = item_index;
        self.measured_items.push_front(ListItem::Measured {
            size,
            focus_handle: focus_handle.clone(),
        });
        focus_handle
    }

    fn push_measured_size_front(
        &mut self,
        item_index: usize,
        item: &ListItem,
        size: Size<Pixels>,
    ) -> Option<FocusHandle> {
        let focus_handle = item.focus_handle();
        self.first_measured_index = item_index;
        self.measured_items.push_front(ListItem::Measured {
            size,
            focus_handle: focus_handle.clone(),
        });
        focus_handle
    }

    fn layout_trailing_items(mut self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        let visible_height = self.rendered_height - self.scroll_top.offset_in_item;
        if visible_height >= self.attempt.bounds.size.height + self.overdraw {
            return self.finish_trailing_items();
        }

        let item_index = self.trailing_index;
        let Some(item) = Self::item_at(&self.old_items, item_index) else {
            return self.finish_trailing_items();
        };
        self.trailing_index += 1;

        let is_visible = visible_height < self.attempt.bounds.size.height;
        if !is_visible && let Some(size) = item.size() {
            self.push_measured_size_back(&item, size);
            return self.layout_trailing_items();
        }

        self.build_item(item_index, item, move |mut this, item, root| {
            let size = root.size();
            this.apply_pending_scroll(item_index, size);
            if is_visible {
                let focus_handle = this.push_measured_back(item_index, &item, root);
                this.check_focus(focus_handle, |this| this.layout_trailing_items())
            } else {
                this.push_measured_size_back(&item, size);
                this.layout_trailing_items()
            }
        })
    }

    fn finish_trailing_items(mut self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        self.rendered_height += self.attempt.padding.bottom;
        self.layout_upward_to_fill()
    }

    fn layout_upward_to_fill(self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        if self.rendered_height - self.scroll_top.offset_in_item >= self.attempt.bounds.size.height
        {
            return self.finish_upward_fill();
        }
        let mut this = self;
        this.reanchor_after_upward_fill = true;
        this.layout_previous_fill_item()
    }

    fn layout_previous_fill_item(mut self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        if self.rendered_height >= self.attempt.bounds.size.height {
            return self.finish_upward_fill();
        }

        let Some(item_index) = self.previous_index.checked_sub(1) else {
            return self.finish_upward_fill();
        };
        let Some(item) = Self::item_at(&self.old_items, item_index) else {
            return self.finish_upward_fill();
        };
        self.previous_index = item_index;

        self.build_item(item_index, item, move |mut this, item, root| {
            let size = root.size();
            this.rendered_height += size.height;
            let focus_handle = this.push_measured_front(item_index, &item, root);
            this.check_focus(focus_handle, |this| this.layout_previous_fill_item())
        })
    }

    fn finish_upward_fill(mut self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        if self.reanchor_after_upward_fill {
            self.scroll_top = ListOffset {
                item_ix: self.previous_index,
                offset_in_item: self.rendered_height - self.attempt.bounds.size.height,
            };

            match self.alignment {
                ListAlignment::Top => {
                    self.scroll_top.offset_in_item = self.scroll_top.offset_in_item.max(px(0.));
                    self.attempt.state.0.borrow_mut().logical_scroll_top = Some(self.scroll_top);
                }
                ListAlignment::Bottom => {
                    self.attempt.state.0.borrow_mut().logical_scroll_top = None;
                }
            }
        }

        self.leading_overdraw = self.scroll_top.offset_in_item;
        self.layout_leading_overdraw()
    }

    fn layout_leading_overdraw(mut self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        if self.leading_overdraw >= self.overdraw {
            return self.commit_measured_items();
        }

        let Some(item_index) = self.previous_index.checked_sub(1) else {
            return self.commit_measured_items();
        };
        let Some(item) = Self::item_at(&self.old_items, item_index) else {
            return self.commit_measured_items();
        };
        self.previous_index = item_index;

        if let Some(size) = item.size() {
            self.leading_overdraw += size.height;
            self.push_measured_size_front(item_index, &item, size);
            return self.layout_leading_overdraw();
        }

        self.build_item(item_index, item, move |mut this, item, root| {
            let size = root.size();
            this.leading_overdraw += size.height;
            this.push_measured_size_front(item_index, &item, size);
            this.layout_leading_overdraw()
        })
    }

    fn commit_measured_items(mut self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        let measured_range =
            self.first_measured_index..(self.first_measured_index + self.measured_items.len());
        let measured_items = std::mem::take(&mut self.measured_items);
        {
            let state = &mut *self.attempt.state.0.borrow_mut();
            let mut cursor = self.old_items.cursor::<Count>(());
            let mut new_items = cursor.slice(&Count(measured_range.start), Bias::Right);
            new_items.extend(measured_items, ());
            cursor.seek(&Count(measured_range.end), Bias::Right);
            new_items.append(cursor.suffix(), ());
            state.items = new_items;

            if state.follow_state.has_stopped_following() {
                let padding = state.last_padding.unwrap_or_default();
                let total_height = state.items.summary().height + padding.top + padding.bottom;
                let scroll_offset = state.scroll_top(&self.scroll_top);
                if scroll_offset + self.attempt.bounds.size.height >= total_height - px(1.0) {
                    state.follow_state.start_following();
                }
            }
        }

        if self.rendered_focused_item {
            return self.clear_autoscroll_before_prepaint();
        }

        let focused_candidates = self
            .attempt
            .state
            .0
            .borrow()
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| item.focus_handle().map(|focus| (index, focus)))
            .collect::<VecDeque<_>>();
        self.find_offscreen_focused_item(focused_candidates)
    }

    fn find_offscreen_focused_item(
        self,
        mut candidates: VecDeque<(usize, FocusHandle)>,
    ) -> CustomLayoutStep<ListCustomLayoutOutput> {
        let Some((item_index, focus_handle)) = candidates.pop_front() else {
            return self.clear_autoscroll_before_prepaint();
        };

        CustomLayoutStep::ContainsFocused {
            focus_handle,
            then: Box::new(move |contains| {
                if !contains {
                    return self.find_offscreen_focused_item(candidates);
                }

                let Some(item) = Self::item_at(&self.attempt.state.0.borrow().items, item_index)
                else {
                    return self.clear_autoscroll_before_prepaint();
                };

                self.build_item(item_index, item, move |mut this, _item, root| {
                    this.item_layouts.push_back(LaidOutItem {
                        index: item_index,
                        root,
                    });
                    this.clear_autoscroll_before_prepaint()
                })
            }),
        }
    }

    fn clear_autoscroll_before_prepaint(self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        CustomLayoutStep::TakeAutoscroll {
            then: Box::new(move |_| self.prepaint_items()),
        }
    }

    fn prepaint_items(self) -> CustomLayoutStep<ListCustomLayoutOutput> {
        if self.attempt.bounds.size.height <= self.attempt.padding.top + self.attempt.padding.bottom
        {
            return self.finish_prepaint(VecDeque::new());
        }

        let mut item_origin =
            self.attempt.bounds.origin + Point::new(px(0.), self.attempt.padding.top);
        if let Some(first_item) = self.item_layouts.front() {
            let state = self.attempt.state.0.borrow();
            let first_item_top = Self::item_start(&state.items, first_item.index);
            item_origin.y += first_item_top - state.scroll_top(&self.scroll_top);
        }

        self.prepaint_next_item(VecDeque::new(), item_origin)
    }

    fn prepaint_next_item(
        mut self,
        mut prepainted_items: VecDeque<PrepaintedItem>,
        item_origin: Point<Pixels>,
    ) -> CustomLayoutStep<ListCustomLayoutOutput> {
        let Some(item) = self.item_layouts.pop_front() else {
            return self.finish_prepaint(prepainted_items);
        };

        let item_index = item.index;
        let item_size = item.root.size();
        let content_mask = Some(ContentMask {
            bounds: self.attempt.bounds,
        });

        CustomLayoutStep::PrepaintVisibleRoot {
            root: item.root,
            origin: item_origin,
            content_mask,
            then: Box::new(move |root, _focus| CustomLayoutStep::TakeAutoscroll {
                then: Box::new(move |autoscroll| {
                    if let Some(autoscroll_bounds) = autoscroll
                        && self.attempt.autoscroll
                        && let Some(scroll_top) =
                            self.autoscroll_scroll_top(item_index, item_origin, autoscroll_bounds)
                    {
                        self.attempt.state.0.borrow_mut().logical_scroll_top = Some(scroll_top);
                        let retry = ListLayoutAttempt {
                            state: self.attempt.state.clone(),
                            bounds: self.attempt.bounds,
                            padding: self.attempt.padding,
                            render_item: self.attempt.render_item.clone(),
                            autoscroll: false,
                        };
                        return CustomLayoutStep::RestartAttempt {
                            next: Box::new(move || retry.start()),
                        };
                    }

                    prepainted_items.push_back(PrepaintedItem { root });
                    self.prepaint_next_item(
                        prepainted_items,
                        Point::new(item_origin.x, item_origin.y + item_size.height),
                    )
                }),
            }),
        }
    }

    fn autoscroll_scroll_top(
        &self,
        item_index: usize,
        item_origin: Point<Pixels>,
        autoscroll_bounds: Bounds<Pixels>,
    ) -> Option<ListOffset> {
        if autoscroll_bounds.top() < self.attempt.bounds.top() {
            return Some(ListOffset {
                item_ix: item_index,
                offset_in_item: autoscroll_bounds.top() - item_origin.y,
            });
        }

        if autoscroll_bounds.bottom() <= self.attempt.bounds.bottom() {
            return None;
        }

        let state = self.attempt.state.0.borrow();
        let mut cursor = state.items.cursor::<Count>(());
        cursor.seek(&Count(item_index), Bias::Right);
        let mut height = self.attempt.bounds.size.height
            - self.attempt.padding.top
            - self.attempt.padding.bottom;
        height -= autoscroll_bounds.bottom() - item_origin.y;

        while height > Pixels::ZERO {
            cursor.prev();
            let Some(item) = cursor.item() else { break };
            let Some(size) = item.size_hint() else { break };
            height -= size.height;
        }

        Some(ListOffset {
            item_ix: cursor.start().0,
            offset_in_item: if height < Pixels::ZERO {
                -height
            } else {
                Pixels::ZERO
            },
        })
    }

    fn finish_prepaint(
        self,
        item_layouts: VecDeque<PrepaintedItem>,
    ) -> CustomLayoutStep<ListCustomLayoutOutput> {
        {
            let state = &mut *self.attempt.state.0.borrow_mut();
            state.last_layout_bounds = Some(self.attempt.bounds);
            state.last_padding = Some(self.attempt.padding);
        }

        CustomLayoutStep::Finish(ListCustomLayoutOutput {
            scroll_top: self.scroll_top,
            item_layouts,
        })
    }
}

#[derive(Clone)]
enum ListItem {
    Unmeasured {
        size_hint: Option<Size<Pixels>>,
        focus_handle: Option<FocusHandle>,
    },
    Measured {
        size: Size<Pixels>,
        focus_handle: Option<FocusHandle>,
    },
}

impl ListItem {
    fn size(&self) -> Option<Size<Pixels>> {
        if let ListItem::Measured { size, .. } = self {
            Some(*size)
        } else {
            None
        }
    }

    fn size_hint(&self) -> Option<Size<Pixels>> {
        match self {
            ListItem::Measured { size, .. } => Some(*size),
            ListItem::Unmeasured { size_hint, .. } => *size_hint,
        }
    }

    fn focus_handle(&self) -> Option<FocusHandle> {
        match self {
            ListItem::Unmeasured { focus_handle, .. } | ListItem::Measured { focus_handle, .. } => {
                focus_handle.clone()
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ListItemSummary {
    count: usize,
    rendered_count: usize,
    unrendered_count: usize,
    height: Pixels,
    has_focus_handles: bool,
    has_unknown_height: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Count(usize);

#[derive(Clone, Debug, Default)]
struct Height(Pixels);

impl ListState {
    /// Construct a new list state, for storage on a view.
    ///
    /// The overdraw parameter controls how much extra space is rendered
    /// above and below the visible area. Elements within this area will
    /// be measured even though they are not visible. This can help ensure
    /// that the list doesn't flicker or pop in when scrolling.
    pub fn new(item_count: usize, alignment: ListAlignment, overdraw: Pixels) -> Self {
        let this = Self(Rc::new(RefCell::new(StateInner {
            last_layout_bounds: None,
            last_padding: None,
            items: SumTree::default(),
            logical_scroll_top: None,
            alignment,
            overdraw,
            scroll_handler: None,
            reset: false,
            scrollbar_drag_start_height: None,
            pending_scroll: None,
            follow_state: FollowState::default(),
        })));
        this.splice(0..0, item_count);
        this
    }

    /// Reset this instantiation of the list state.
    ///
    /// Note that this will cause scroll events to be dropped until the next paint.
    pub fn reset(&self, element_count: usize) {
        let old_count = {
            let state = &mut *self.0.borrow_mut();
            state.reset = true;
            state.logical_scroll_top = None;
            state.scrollbar_drag_start_height = None;
            state.items.summary().count
        };

        self.splice(0..old_count, element_count);
    }

    /// Remeasure all items while preserving proportional scroll position.
    ///
    /// Use this when item heights may have changed (e.g., font size changes)
    /// but the number and identity of items remains the same.
    pub fn remeasure(&self) {
        let count = self.item_count();
        self.remeasure_items_with_scroll_anchor(0..count, ScrollAnchor::Proportional);
    }

    /// Mark items in `range` as needing remeasurement while preserving
    /// the current scroll position. Unlike [`Self::splice`], this does
    /// not change the number of items or blow away `logical_scroll_top`.
    ///
    /// Use this when an item's content has changed and its rendered
    /// height may be different (e.g., streaming text, tool results
    /// loading), but the item itself still exists at the same index.
    pub fn remeasure_items(&self, range: Range<usize>) {
        self.remeasure_items_with_scroll_anchor(range, ScrollAnchor::Absolute);
    }

    /// Provide explicit size facts for existing items.
    ///
    /// Size hints let list owners preserve scrollbar and scroll-position math
    /// for offscreen items without asking GPUI to render arbitrary scratch
    /// elements. The list treats these as caller-owned facts until an item is
    /// rendered as a visible root and produces a measured size.
    pub fn set_size_hints(
        &self,
        range: Range<usize>,
        sizes: impl IntoIterator<Item = Size<Pixels>>,
    ) {
        let sizes = sizes.into_iter().collect::<Vec<_>>();
        assert_eq!(
            range.len(),
            sizes.len(),
            "size hint count must match the target item range"
        );

        let state = &mut *self.0.borrow_mut();
        let mut cursor = state.items.cursor::<Count>(());
        let mut new_items = cursor.slice(&Count(range.start), Bias::Right);
        let hinted = cursor.slice(&Count(range.end), Bias::Right);
        new_items.extend(
            hinted
                .iter()
                .zip(sizes)
                .map(|(item, size)| ListItem::Unmeasured {
                    size_hint: Some(size),
                    focus_handle: item.focus_handle(),
                }),
            (),
        );
        new_items.append(cursor.suffix(), ());
        drop(cursor);
        state.items = new_items;
    }

    fn remeasure_items_with_scroll_anchor(&self, range: Range<usize>, scroll_anchor: ScrollAnchor) {
        let state = &mut *self.0.borrow_mut();

        if let Some(scroll_top) = state.logical_scroll_top {
            if range.contains(&scroll_top.item_ix) {
                state.pending_scroll = match scroll_anchor {
                    ScrollAnchor::Absolute => Some(PendingScroll::Absolute {
                        item_ix: scroll_top.item_ix,
                        offset: scroll_top.offset_in_item,
                    }),
                    ScrollAnchor::Proportional => {
                        // If the scroll-top item falls within the remeasured range,
                        // store a fractional offset so the layout can restore the
                        // proportional scroll position after the item is re-rendered
                        // at its new height.
                        let mut cursor = state.items.cursor::<Count>(());
                        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);

                        cursor
                            .item()
                            .and_then(|item| {
                                item.size().map(|size| {
                                    let fraction = if size.height.0 > 0.0 {
                                        (scroll_top.offset_in_item.0 / size.height.0)
                                            .clamp(0.0, 1.0)
                                    } else {
                                        0.0
                                    };

                                    PendingScroll::Proportional(PendingScrollFraction {
                                        item_ix: scroll_top.item_ix,
                                        fraction,
                                    })
                                })
                            })
                            .or_else(|| state.pending_scroll.clone())
                    }
                };
            }
        }

        // Rebuild the tree, replacing items in the range with
        // Unmeasured copies that keep their focus handles.
        let new_items = {
            let mut cursor = state.items.cursor::<Count>(());
            let mut new_items = cursor.slice(&Count(range.start), Bias::Right);
            let invalidated = cursor.slice(&Count(range.end), Bias::Right);
            new_items.extend(
                invalidated.iter().map(|item| ListItem::Unmeasured {
                    size_hint: item.size_hint(),
                    focus_handle: item.focus_handle(),
                }),
                (),
            );
            new_items.append(cursor.suffix(), ());
            new_items
        };
        state.items = new_items;
    }

    /// The number of items in this list.
    pub fn item_count(&self) -> usize {
        self.0.borrow().items.summary().count
    }

    /// Whether the list is scrolled to the end, or `None` if the list is
    /// not scrollable or the total content height is not yet known.
    pub fn is_scrolled_to_end(&self) -> Option<bool> {
        let state = self.0.borrow();
        let bounds = state.last_layout_bounds?;
        let summary = state.items.summary();
        if summary.has_unknown_height {
            return None;
        }
        let padding = state.last_padding.unwrap_or_default();
        let content_height = summary.height + padding.top + padding.bottom;
        let scroll_max = (content_height - bounds.size.height).max(px(0.));
        if scroll_max <= px(0.) {
            return None;
        }
        let scroll_top = state.scroll_top(&state.logical_scroll_top());
        Some(scroll_top >= scroll_max)
    }

    /// Inform the list state that the items in `old_range` have been replaced
    /// by `count` new items that must be recalculated.
    pub fn splice(&self, old_range: Range<usize>, count: usize) {
        self.splice_focusable(old_range, (0..count).map(|_| None))
    }

    /// Register with the list state that the items in `old_range` have been replaced
    /// by new items. As opposed to [`Self::splice`], this method allows an iterator of optional focus handles
    /// to be supplied to properly integrate with items in the list that can be focused. If a focused item
    /// is scrolled out of view, the list will continue to render it to allow keyboard interaction.
    pub fn splice_focusable(
        &self,
        old_range: Range<usize>,
        focus_handles: impl IntoIterator<Item = Option<FocusHandle>>,
    ) {
        let state = &mut *self.0.borrow_mut();

        let mut old_items = state.items.cursor::<Count>(());
        let mut new_items = old_items.slice(&Count(old_range.start), Bias::Right);
        old_items.seek_forward(&Count(old_range.end), Bias::Right);

        let mut spliced_count = 0;
        new_items.extend(
            focus_handles.into_iter().map(|focus_handle| {
                spliced_count += 1;
                ListItem::Unmeasured {
                    size_hint: None,
                    focus_handle,
                }
            }),
            (),
        );
        new_items.append(old_items.suffix(), ());
        drop(old_items);
        state.items = new_items;

        if let Some(ListOffset {
            item_ix,
            offset_in_item,
        }) = state.logical_scroll_top.as_mut()
        {
            if old_range.contains(item_ix) {
                *item_ix = old_range.start;
                *offset_in_item = px(0.);
            } else if old_range.end <= *item_ix {
                *item_ix = *item_ix - (old_range.end - old_range.start) + spliced_count;
            }
        }
    }

    /// Set a handler that will be called when the list is scrolled.
    pub fn set_scroll_handler(
        &self,
        handler: impl FnMut(&ListScrollEvent, &mut Window, &mut App) + 'static,
    ) {
        self.0.borrow_mut().scroll_handler = Some(Box::new(handler))
    }

    /// Get the current scroll offset, in terms of the list's items.
    pub fn logical_scroll_top(&self) -> ListOffset {
        self.0.borrow().logical_scroll_top()
    }

    /// Scroll the list by the given offset
    pub fn scroll_by(&self, distance: Pixels) {
        if distance == px(0.) {
            return;
        }

        let current_offset = self.logical_scroll_top();
        let state = &mut *self.0.borrow_mut();

        if distance < px(0.) {
            state.follow_state.stop_following();
        }

        let mut cursor = state.items.cursor::<ListItemSummary>(());
        cursor.seek(&Count(current_offset.item_ix), Bias::Right);

        let start_pixel_offset = cursor.start().height + current_offset.offset_in_item;
        let new_pixel_offset = (start_pixel_offset + distance).max(px(0.));
        if new_pixel_offset > start_pixel_offset {
            cursor.seek_forward(&Height(new_pixel_offset), Bias::Right);
        } else {
            cursor.seek(&Height(new_pixel_offset), Bias::Right);
        }

        state.logical_scroll_top = Some(ListOffset {
            item_ix: cursor.start().count,
            offset_in_item: new_pixel_offset - cursor.start().height,
        });
    }

    /// Scroll the list to the very end (past the last item).
    ///
    /// Unlike [`scroll_to_reveal_item`], this uses the total item count as the
    /// anchor, so the list's layout pass will walk backwards from the end and
    /// always show the bottom of the last item — even when that item is still
    /// growing (e.g. during streaming).
    pub fn scroll_to_end(&self) {
        let state = &mut *self.0.borrow_mut();
        let item_count = state.items.summary().count;
        state.logical_scroll_top = Some(ListOffset {
            item_ix: item_count,
            offset_in_item: px(0.),
        });
    }

    /// Set the follow mode for the list. In `Tail` mode, the list
    /// will auto-scroll to the end and re-engage after the user
    /// scrolls back to the bottom. In `Normal` mode, no automatic
    /// following occurs.
    pub fn set_follow_mode(&self, mode: FollowMode) {
        let state = &mut *self.0.borrow_mut();

        match mode {
            FollowMode::Normal => {
                state.follow_state = FollowState::Normal;
            }
            FollowMode::Tail => {
                state.follow_state = FollowState::Tail { is_following: true };
                if matches!(mode, FollowMode::Tail) {
                    let item_count = state.items.summary().count;
                    state.logical_scroll_top = Some(ListOffset {
                        item_ix: item_count,
                        offset_in_item: px(0.),
                    });
                }
            }
        }
    }

    /// Returns whether the list is currently actively following the
    /// tail (snapping to the end on each layout).
    pub fn is_following_tail(&self) -> bool {
        matches!(
            self.0.borrow().follow_state,
            FollowState::Tail { is_following: true }
        )
    }

    /// Scroll the list to the given offset
    pub fn scroll_to(&self, mut scroll_top: ListOffset) {
        let state = &mut *self.0.borrow_mut();
        let item_count = state.items.summary().count;
        if scroll_top.item_ix >= item_count {
            scroll_top.item_ix = item_count;
            scroll_top.offset_in_item = px(0.);
        }

        if scroll_top.item_ix < item_count {
            state.follow_state.stop_following();
        }

        state.logical_scroll_top = Some(scroll_top);
    }

    /// Scroll the list to the given item, such that the item is fully visible.
    pub fn scroll_to_reveal_item(&self, ix: usize) {
        let state = &mut *self.0.borrow_mut();

        let mut scroll_top = state.logical_scroll_top();
        let height = state
            .last_layout_bounds
            .map_or(px(0.), |bounds| bounds.size.height);
        let padding = state.last_padding.unwrap_or_default();

        if ix <= scroll_top.item_ix {
            scroll_top.item_ix = ix;
            scroll_top.offset_in_item = px(0.);
        } else {
            let mut cursor = state.items.cursor::<ListItemSummary>(());
            cursor.seek(&Count(ix + 1), Bias::Right);
            let bottom = cursor.start().height + padding.top;
            let goal_top = px(0.).max(bottom - height + padding.bottom);

            cursor.seek(&Height(goal_top), Bias::Left);
            let start_ix = cursor.start().count;
            let start_item_top = cursor.start().height;

            if start_ix >= scroll_top.item_ix {
                scroll_top.item_ix = start_ix;
                scroll_top.offset_in_item = goal_top - start_item_top;
            }
        }

        state.logical_scroll_top = Some(scroll_top);
    }

    /// Get the bounds for the given item in window coordinates, if it's
    /// been rendered.
    pub fn bounds_for_item(&self, ix: usize) -> Option<Bounds<Pixels>> {
        let state = &*self.0.borrow();

        let bounds = state.last_layout_bounds.unwrap_or_default();
        let scroll_top = state.logical_scroll_top();
        if ix < scroll_top.item_ix {
            return None;
        }

        let mut cursor = state.items.cursor::<Dimensions<Count, Height>>(());
        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);

        let scroll_top = cursor.start().1.0 + scroll_top.offset_in_item;

        cursor.seek_forward(&Count(ix), Bias::Right);
        if let Some(&ListItem::Measured { size, .. }) = cursor.item() {
            let &Dimensions(Count(count), Height(top), _) = cursor.start();
            if count == ix {
                let top = bounds.top() + top - scroll_top;
                return Some(Bounds::from_corners(
                    point(bounds.left(), top),
                    point(bounds.right(), top + size.height),
                ));
            }
        }
        None
    }

    /// Call this method when the user starts dragging the scrollbar.
    ///
    /// This will prevent the height reported to the scrollbar from changing during the drag
    /// as items in the overdraw get measured, and help offset scroll position changes accordingly.
    pub fn scrollbar_drag_started(&self) {
        let mut state = self.0.borrow_mut();
        state.scrollbar_drag_start_height = Some(state.items.summary().height);
    }

    /// Called when the user stops dragging the scrollbar.
    ///
    /// See `scrollbar_drag_started`.
    pub fn scrollbar_drag_ended(&self) {
        self.0.borrow_mut().scrollbar_drag_start_height.take();
    }

    /// Returns `true` if the scrollbar is currently being dragged.
    ///
    /// This is set between [`scrollbar_drag_started`](Self::scrollbar_drag_started)
    /// and [`scrollbar_drag_ended`](Self::scrollbar_drag_ended) calls. Useful for
    /// consumers that need to distinguish scrollbar drags from wheel/trackpad scrolls,
    /// e.g. to suppress auto-scroll behavior during manual positioning.
    pub fn is_scrollbar_dragging(&self) -> bool {
        self.0.borrow().scrollbar_drag_start_height.is_some()
    }

    /// Set the offset from the scrollbar
    pub fn set_offset_from_scrollbar(&self, point: Point<Pixels>) {
        self.0.borrow_mut().set_offset_from_scrollbar(point);
    }

    /// Returns the maximum scroll offset according to the items we have measured.
    /// This value remains constant while dragging to prevent the scrollbar from moving away unexpectedly.
    pub fn max_offset_for_scrollbar(&self) -> Point<Pixels> {
        let state = self.0.borrow();
        point(Pixels::ZERO, state.max_scroll_offset())
    }

    /// Returns the current scroll offset adjusted for the scrollbar.
    ///
    /// The returned offset has a negative `y` component representing
    /// how far the content has scrolled.
    pub fn scroll_px_offset_for_scrollbar(&self) -> Point<Pixels> {
        let state = &self.0.borrow();

        if state.logical_scroll_top.is_none() && state.alignment == ListAlignment::Bottom {
            return Point::new(px(0.), -state.max_scroll_offset());
        }

        let logical_scroll_top = state.logical_scroll_top();

        let mut cursor = state.items.cursor::<ListItemSummary>(());
        let summary: ListItemSummary =
            cursor.summary(&Count(logical_scroll_top.item_ix), Bias::Right);
        let offset = summary.height + logical_scroll_top.offset_in_item;

        Point::new(px(0.), -offset)
    }

    /// Return the bounds of the viewport in pixels.
    pub fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.0.borrow().last_layout_bounds.unwrap_or_default()
    }
}

impl StateInner {
    fn known_layout_summary(&self) -> (Pixels, Pixels) {
        let mut max_item_width = px(0.);
        for item in self.items.iter() {
            if let Some(size) = item.size_hint() {
                max_item_width = max_item_width.max(size.width);
            }
        }

        (max_item_width, self.items.summary().height)
    }

    fn max_scroll_offset(&self) -> Pixels {
        let bounds = self.last_layout_bounds.unwrap_or_default();
        let height = self
            .scrollbar_drag_start_height
            .unwrap_or_else(|| self.items.summary().height);
        (height - bounds.size.height).max(px(0.))
    }

    fn visible_range(
        items: &SumTree<ListItem>,
        height: Pixels,
        scroll_top: &ListOffset,
    ) -> Range<usize> {
        let mut cursor = items.cursor::<ListItemSummary>(());
        cursor.seek(&Count(scroll_top.item_ix), Bias::Right);
        let start_y = cursor.start().height + scroll_top.offset_in_item;
        cursor.seek_forward(&Height(start_y + height), Bias::Left);
        scroll_top.item_ix..cursor.start().count + 1
    }

    fn scroll(
        &mut self,
        scroll_top: &ListOffset,
        height: Pixels,
        delta: Point<Pixels>,
        current_view: EntityId,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Drop scroll events after a reset, since we can't calculate
        // the new logical scroll top without the item heights
        if self.reset {
            return;
        }

        let padding = self.last_padding.unwrap_or_default();
        let scroll_max =
            (self.items.summary().height + padding.top + padding.bottom - height).max(px(0.));
        let new_scroll_top = (self.scroll_top(scroll_top) - delta.y)
            .max(px(0.))
            .min(scroll_max);

        if self.alignment == ListAlignment::Bottom && new_scroll_top == scroll_max {
            self.logical_scroll_top = None;
        } else {
            let (start, ..) =
                self.items
                    .find::<ListItemSummary, _>((), &Height(new_scroll_top), Bias::Right);
            let item_ix = start.count;
            let offset_in_item = new_scroll_top - start.height;
            self.logical_scroll_top = Some(ListOffset {
                item_ix,
                offset_in_item,
            });
        }

        if delta.y > px(0.) {
            self.follow_state.stop_following();
        }

        if let Some(handler) = self.scroll_handler.as_mut() {
            let visible_range = Self::visible_range(&self.items, height, scroll_top);
            handler(
                &ListScrollEvent {
                    visible_range,
                    count: self.items.summary().count,
                    is_scrolled: self.logical_scroll_top.is_some(),
                    is_following_tail: matches!(
                        self.follow_state,
                        FollowState::Tail { is_following: true }
                    ),
                },
                window,
                cx,
            );
        }

        cx.notify(current_view);
    }

    fn logical_scroll_top(&self) -> ListOffset {
        self.logical_scroll_top
            .unwrap_or_else(|| match self.alignment {
                ListAlignment::Top => ListOffset {
                    item_ix: 0,
                    offset_in_item: px(0.),
                },
                ListAlignment::Bottom => ListOffset {
                    item_ix: self.items.summary().count,
                    offset_in_item: px(0.),
                },
            })
    }

    fn scroll_top(&self, logical_scroll_top: &ListOffset) -> Pixels {
        let (start, ..) = self.items.find::<ListItemSummary, _>(
            (),
            &Count(logical_scroll_top.item_ix),
            Bias::Right,
        );
        start.height + logical_scroll_top.offset_in_item
    }

    // Scrollbar support

    fn set_offset_from_scrollbar(&mut self, point: Point<Pixels>) {
        let Some(bounds) = self.last_layout_bounds else {
            return;
        };
        let height = bounds.size.height;

        let padding = self.last_padding.unwrap_or_default();
        // Scrollbar drag positions are computed from the content height
        // captured at drag start, so map them back using the same height.
        let content_height = self
            .scrollbar_drag_start_height
            .unwrap_or_else(|| self.items.summary().height);
        let scroll_max = (content_height + padding.top + padding.bottom - height).max(px(0.));
        let new_scroll_top = (-point.y).max(px(0.)).min(scroll_max);

        // If content grew during the drag, the frozen bottom is below the
        // live bottom. Treat dragging to the frozen end as resuming tail follow.
        let dragged_to_end =
            scroll_max > px(0.) && new_scroll_top >= (scroll_max - px(1.0)).max(px(0.));
        if dragged_to_end && matches!(self.follow_state, FollowState::Tail { .. }) {
            self.follow_state = FollowState::Tail { is_following: true };
            let item_count = self.items.summary().count;
            self.logical_scroll_top = Some(ListOffset {
                item_ix: item_count,
                offset_in_item: px(0.),
            });
            return;
        }

        self.follow_state.stop_following();

        if self.alignment == ListAlignment::Bottom && new_scroll_top == scroll_max {
            self.logical_scroll_top = None;
        } else {
            let (start, _, _) =
                self.items
                    .find::<ListItemSummary, _>((), &Height(new_scroll_top), Bias::Right);

            let item_ix = start.count;
            let offset_in_item = new_scroll_top - start.height;
            self.logical_scroll_top = Some(ListOffset {
                item_ix,
                offset_in_item,
            });
        }
    }
}

impl std::fmt::Debug for ListItem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unmeasured { .. } => write!(f, "Unrendered"),
            Self::Measured { size, .. } => f.debug_struct("Rendered").field("size", size).finish(),
        }
    }
}

/// An offset into the list's items, in terms of the item index and the number
/// of pixels off the top left of the item.
#[derive(Debug, Clone, Copy, Default)]
pub struct ListOffset {
    /// The index of an item in the list
    pub item_ix: usize,
    /// The number of pixels to offset from the item index.
    pub offset_in_item: Pixels,
}

impl Element for List {
    type RequestLayoutState = ();
    type PrepaintState = ListPrepaintState;

    fn id(&self) -> Option<crate::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut LayoutRequestCx<'_>,
        cx: &mut App,
    ) -> (crate::LayoutId, Self::RequestLayoutState) {
        let layout_id = match self.sizing_behavior {
            ListSizingBehavior::Infer => {
                let mut style = Style::default();
                style.overflow.y = Overflow::Scroll;
                style.refine(&self.style);
                window.with_text_style(style.text_style().cloned(), |window| {
                    let state = &mut *self.state.0.borrow_mut();

                    let (max_element_width, total_height) = state.known_layout_summary();

                    window.request_pure_measured_layout(
                        style,
                        PureSizeMeasure::list(
                            max_element_width,
                            total_height,
                            window.scale_factor(),
                        ),
                    )
                })
            }
            ListSizingBehavior::Auto => {
                let mut style = Style::default();
                style.refine(&self.style);
                window.with_text_style(style.text_style().cloned(), |window| {
                    window.request_layout(style, None, cx)
                })
            }
        };
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut PrepaintCx<'_>,
        _cx: &mut App,
    ) -> ListPrepaintState {
        let state = &mut *self.state.0.borrow_mut();
        state.reset = false;

        let mut style = Style::default();
        style.refine(&self.style);

        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);

        // If the width of the list has changed, invalidate all cached item heights
        if state
            .last_layout_bounds
            .is_none_or(|last_bounds| last_bounds.size.width != bounds.size.width)
        {
            let new_items = SumTree::from_iter(
                state.items.iter().map(|item| ListItem::Unmeasured {
                    size_hint: item.size_hint(),
                    focus_handle: item.focus_handle(),
                }),
                (),
            );

            state.items = new_items;
        }

        let padding = style
            .padding
            .to_pixels(bounds.size.into(), window.rem_size());
        let work = ListCustomLayoutJob::new(
            self.state.clone(),
            bounds,
            padding,
            self.render_item.clone(),
        );
        let layout = window.register_custom_layout(move || work.run());

        ListPrepaintState { hitbox, layout }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<crate::Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut PaintCx<'_>,
        cx: &mut App,
    ) {
        let current_view = window.current_view();
        let mut layout = window.take_frame_output(prepaint.layout);
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for item in layout.item_layouts.drain(..) {
                window.paint_custom_layout_root(item.root, cx);
            }
        });

        let list_state = self.state.clone();
        let height = bounds.size.height;
        let scroll_top = layout.scroll_top;
        let hitbox_id = prepaint.hitbox.id;
        let mut accumulated_scroll_delta = ScrollDelta::default();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Bubble && hitbox_id.should_handle_scroll(window) {
                accumulated_scroll_delta = accumulated_scroll_delta.coalesce(event.delta);
                let pixel_delta = accumulated_scroll_delta.pixel_delta(px(20.));
                list_state.0.borrow_mut().scroll(
                    &scroll_top,
                    height,
                    pixel_delta,
                    current_view,
                    window,
                    cx,
                )
            }
        });
    }
}

impl IntoElement for List {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for List {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl sum_tree::Item for ListItem {
    type Summary = ListItemSummary;

    fn summary(&self, _: ()) -> Self::Summary {
        match self {
            ListItem::Unmeasured {
                size_hint,
                focus_handle,
            } => ListItemSummary {
                count: 1,
                rendered_count: 0,
                unrendered_count: 1,
                height: if let Some(size) = size_hint {
                    size.height
                } else {
                    px(0.)
                },
                has_focus_handles: focus_handle.is_some(),
                has_unknown_height: size_hint.is_none(),
            },
            ListItem::Measured {
                size, focus_handle, ..
            } => ListItemSummary {
                count: 1,
                rendered_count: 1,
                unrendered_count: 0,
                height: size.height,
                has_focus_handles: focus_handle.is_some(),
                has_unknown_height: false,
            },
        }
    }
}

impl sum_tree::ContextLessSummary for ListItemSummary {
    fn zero() -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &Self) {
        self.count += summary.count;
        self.rendered_count += summary.rendered_count;
        self.unrendered_count += summary.unrendered_count;
        self.height += summary.height;
        self.has_focus_handles |= summary.has_focus_handles;
        self.has_unknown_height |= summary.has_unknown_height;
    }
}

impl<'a> sum_tree::Dimension<'a, ListItemSummary> for Count {
    fn zero(_cx: ()) -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &'a ListItemSummary, _: ()) {
        self.0 += summary.count;
    }
}

impl<'a> sum_tree::Dimension<'a, ListItemSummary> for Height {
    fn zero(_cx: ()) -> Self {
        Default::default()
    }

    fn add_summary(&mut self, summary: &'a ListItemSummary, _: ()) {
        self.0 += summary.height;
    }
}

impl sum_tree::SeekTarget<'_, ListItemSummary, ListItemSummary> for Count {
    fn cmp(&self, other: &ListItemSummary, _: ()) -> std::cmp::Ordering {
        self.0.partial_cmp(&other.count).unwrap()
    }
}

impl sum_tree::SeekTarget<'_, ListItemSummary, ListItemSummary> for Height {
    fn cmp(&self, other: &ListItemSummary, _: ()) -> std::cmp::Ordering {
        self.0.partial_cmp(&other.height).unwrap()
    }
}

#[cfg(test)]
mod test {

    use gpui::{ScrollDelta, ScrollWheelEvent};
    use std::cell::Cell;
    use std::rc::Rc;

    use crate::{
        self as gpui, AppContext, Context, Element, FollowMode, IntoElement, ListState, Render,
        Styled, TestAppContext, Window, div, list, point, px, size,
    };

    #[gpui::test]
    fn test_reset_after_paint_before_scroll(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));

        // Ensure that the list is scrolled to the top
        state.scroll_to(gpui::ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(10.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        // Paint
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(20.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        // Reset
        state.reset(5);

        // And then receive a scroll event _before_ the next paint
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(1.), px(1.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-500.))),
            ..Default::default()
        });

        // Scroll position should stay at the top of the list
        assert_eq!(state.logical_scroll_top().item_ix, 0);
        assert_eq!(state.logical_scroll_top().offset_in_item, px(0.));
    }

    #[gpui::test]
    fn test_scroll_by_positive_and_negative_distance(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let state = ListState::new(5, crate::ListAlignment::Top, px(10.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(20.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        // Paint
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        // Test positive distance: start at item 1, move down 30px
        state.scroll_by(px(30.));

        // Should move to item 2
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 1);
        assert_eq!(offset.offset_in_item, px(10.));

        // Test negative distance: start at item 2, move up 30px
        state.scroll_by(px(-30.));

        // Should move back to item 1
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 0);
        assert_eq!(offset.offset_in_item, px(0.));

        // Test zero distance
        state.scroll_by(px(0.));
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 0);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    #[gpui::test]
    fn test_remeasure(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // Create a list with 10 items, each 100px tall. We'll keep a reference
        // to the item height so we can later change the height and assert how
        // `ListState` handles it.
        let item_height = Rc::new(Cell::new(100usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(10.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }

        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |_, _, _| {
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let state_clone = state.clone();
        let item_height_clone = item_height.clone();
        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state_clone,
                item_height: item_height_clone,
            })
        });

        // Simulate scrolling 40px inside the element with index 2. Since the
        // original item height is 100px, this equates to 40% inside the item.
        state.scroll_to(gpui::ListOffset {
            item_ix: 2,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 2);
        assert_eq!(offset.offset_in_item, px(40.));

        // Update the `item_height` to be 50px instead of 100px so we can assert
        // that the scroll position is proportionally preserved, that is,
        // instead of 40px from the top of item 2, it should be 20px, since the
        // item's height has been halved.
        item_height.set(50);
        state.remeasure();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 2);
        assert_eq!(offset.offset_in_item, px(20.));
    }

    #[gpui::test]
    fn test_remeasure_item_preserves_scroll_offset(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let item_height = Rc::new(Cell::new(100usize));
        let state = ListState::new(20, crate::ListAlignment::Top, px(10.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }

        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |index, _, _| {
                    let height = if index == 5 { height } else { 100 };
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let state_clone = state.clone();
        let item_height_clone = item_height.clone();
        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state_clone,
                item_height: item_height_clone,
            })
        });

        state.scroll_to(gpui::ListOffset {
            item_ix: 5,
            offset_in_item: px(40.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        item_height.set(200);
        state.remeasure_items(5..6);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 5);
        assert_eq!(offset.offset_in_item, px(40.));
    }

    #[gpui::test]
    fn test_follow_tail_stays_at_bottom_as_items_grow(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items, each 50px tall → 500px total content, 200px viewport.
        // With follow-tail on, the list should always show the bottom.
        let item_height = Rc::new(Cell::new(50usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView {
            state: ListState,
            item_height: Rc<Cell<usize>>,
        }
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                let height = self.item_height.get();
                list(self.state.clone(), move |_, _, _| {
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let state_clone = state.clone();
        let item_height_clone = item_height.clone();
        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state_clone,
                item_height: item_height_clone,
            })
        });

        state.set_follow_mode(FollowMode::Tail);

        // First paint — items are 50px, total 500px, viewport 200px.
        // Follow-tail should anchor to the end.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        // The scroll should be at the bottom: the last visible items fill the
        // 200px viewport from the end of 500px of content (offset 300px).
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 6);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(state.is_following_tail());

        // Simulate items growing (e.g. streaming content makes each item taller).
        // 10 items × 80px = 800px total.
        item_height.set(80);
        state.remeasure();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        // After growth, follow-tail should have re-anchored to the new end.
        // 800px total − 200px viewport = 600px offset → item 7 at offset 40px,
        // but follow-tail anchors to item_count (10), and layout walks back to
        // fill 200px, landing at item 7 (7 × 80 = 560, 800 − 560 = 240 > 200,
        // so item 8: 8 × 80 = 640, 800 − 640 = 160 < 200 → keeps walking →
        // item 7: offset = 800 − 200 = 600, item_ix = 600/80 = 7, remainder 40).
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 7);
        assert_eq!(offset.offset_in_item, px(40.));
        assert!(state.is_following_tail());
    }

    #[gpui::test]
    fn test_follow_tail_disengages_on_user_scroll(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        state.set_follow_mode(FollowMode::Tail);

        // Paint with follow-tail — scroll anchored to the bottom.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });
        assert!(state.is_following_tail());

        // Simulate the user scrolling up.
        // This should disengage follow-tail.
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(100.))),
            ..Default::default()
        });

        assert!(
            !state.is_following_tail(),
            "follow-tail should disengage when the user scrolls toward the start"
        );
    }

    #[gpui::test]
    fn test_follow_tail_disengages_on_scrollbar_reposition(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));
        state.set_size_hints(0..10, std::iter::repeat_n(size(px(100.), px(50.)), 10));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);

        // Paint with follow-tail — scroll anchored to the bottom.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        // Simulate the scrollbar moving the viewport to the middle.
        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(
            !state.is_following_tail(),
            "follow-tail should disengage when the scrollbar manually repositions the list"
        );

        // A subsequent draw should preserve the user's manual position instead
        // of snapping back to the end.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    #[gpui::test]
    fn test_scrollbar_drag_with_growing_content(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        let last_item_height = Rc::new(Cell::new(50usize));
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));
        state.set_size_hints(0..10, std::iter::repeat_n(size(px(100.), px(50.)), 10));

        struct TestView {
            state: ListState,
            last_item_height: Rc<Cell<usize>>,
        }
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                let last_item_height = self.last_item_height.clone();
                list(self.state.clone(), move |index, _, _| {
                    let height = if index == 9 {
                        last_item_height.get()
                    } else {
                        50
                    };
                    div().h(px(height as f32)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| {
            cx.new(|_| TestView {
                state: state.clone(),
                last_item_height: last_item_height.clone(),
            })
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        state.scrollbar_drag_started();

        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));
        let scrollbar_offset_before_growth = state.scroll_px_offset_for_scrollbar();

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));

        last_item_height.set(550);
        state.remeasure_items(9..10);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        assert_eq!(state.max_offset_for_scrollbar().y, px(300.));
        assert_eq!(
            state.scroll_px_offset_for_scrollbar(),
            scrollbar_offset_before_growth
        );

        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
    }

    #[gpui::test]
    fn test_set_follow_tail_snaps_to_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        // Scroll to the middle of the list (item 3).
        state.scroll_to(gpui::ListOffset {
            item_ix: 3,
            offset_in_item: px(0.),
        });

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 3);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(!state.is_following_tail());

        // Enable follow-tail — this should immediately snap the scroll anchor
        // to the end, like the user just sent a prompt.
        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        // After paint, scroll should be at the bottom.
        // 500px total − 200px viewport = 300px offset → item 6, offset 0.
        let offset = state.logical_scroll_top();
        assert_eq!(offset.item_ix, 6);
        assert_eq!(offset.offset_in_item, px(0.));
        assert!(state.is_following_tail());
    }

    #[gpui::test]
    fn test_bottom_aligned_scrollbar_offset_at_end(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        const ITEMS: usize = 10;
        const ITEM_SIZE: f32 = 50.0;

        let state = ListState::new(
            ITEMS,
            crate::ListAlignment::Bottom,
            px(ITEMS as f32 * ITEM_SIZE),
        );

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(ITEM_SIZE)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(100.)), |_, cx| {
            cx.new(|_| TestView(state.clone())).into_any_element()
        });

        // Bottom-aligned lists start pinned to the end: logical_scroll_top returns
        // item_ix == item_count, meaning no explicit scroll position has been set.
        assert_eq!(state.logical_scroll_top().item_ix, ITEMS);

        let max_offset = state.max_offset_for_scrollbar();
        let scroll_offset = state.scroll_px_offset_for_scrollbar();

        assert_eq!(
            -scroll_offset.y, max_offset.y,
            "scrollbar offset ({}) should equal max offset ({}) when list is pinned to bottom",
            -scroll_offset.y, max_offset.y,
        );
    }

    /// When the user scrolls away from the bottom during follow_tail,
    /// follow_tail suspends. If they scroll back to the bottom, the
    /// next paint should re-engage follow_tail using fresh measurements.
    #[gpui::test]
    fn test_follow_tail_reengages_when_scrolled_back_to_bottom(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        // Scroll up — follow_tail should suspend (not fully disengage).
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(50.))),
            ..Default::default()
        });
        assert!(!state.is_following_tail());

        // Scroll back down to the bottom.
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(-10000.))),
            ..Default::default()
        });

        // After a paint, follow_tail should re-engage because the
        // layout confirmed we're at the true bottom.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "follow_tail should re-engage after scrolling back to the bottom"
        );
    }

    /// When an item is spliced to unmeasured (0px) while follow_tail
    /// is suspended, the re-engagement check should still work correctly
    #[gpui::test]
    fn test_follow_tail_reengagement_not_fooled_by_unmeasured_items(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 20 items × 50px = 1000px total, 200px viewport, 1000px
        // overdraw so all items get measured during the follow_tail
        // paint (matching realistic production settings).
        let state = ListState::new(20, crate::ListAlignment::Top, px(1000.));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        // Scroll up a meaningful amount — suspends follow_tail.
        // 20 items × 50px = 1000px. viewport 200px. scroll_max = 800px.
        // Scrolling up 200px puts us at 600px, clearly not at bottom.
        cx.simulate_event(ScrollWheelEvent {
            position: point(px(50.), px(100.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(200.))),
            ..Default::default()
        });
        assert!(!state.is_following_tail());

        // Invalidate the last item (simulates EntryUpdated calling
        // remeasure_items). This makes items.summary().height
        // temporarily wrong (0px for the invalidated item).
        state.remeasure_items(19..20);

        // Paint — layout re-measures the invalidated item with its true
        // height. The re-engagement check uses these fresh measurements.
        // Since we scrolled 200px up from the 800px max, we're at
        // ~600px — NOT at the bottom, so follow_tail should NOT
        // re-engage.
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(
            !state.is_following_tail(),
            "follow_tail should not falsely re-engage due to an unmeasured item \
             reducing items.summary().height"
        );
    }

    #[gpui::test]
    fn test_follow_tail_reengages_after_scrollbar_disengagement(cx: &mut TestAppContext) {
        let cx = cx.add_empty_window();

        // 10 items × 50px = 500px total, 200px viewport, scroll_max = 300px.
        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));
        state.set_size_hints(0..10, std::iter::repeat_n(size(px(100.), px(50.)), 10));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        // Drag the scrollbar up to the middle — follow_tail should suspend.
        state.set_offset_from_scrollbar(point(px(0.), px(-150.)));
        assert!(!state.is_following_tail());

        // Drag the scrollbar back to the bottom — follow_tail should re-engage
        // on the next paint.
        state.set_offset_from_scrollbar(point(px(0.), px(-300.)));
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });
        assert!(
            state.is_following_tail(),
            "follow_tail should re-engage after scrolling back to the bottom via the scrollbar"
        );
    }

    #[gpui::test]
    fn test_follow_tail_reengages_after_scrollbar_drag_to_bottom_while_growing(
        cx: &mut TestAppContext,
    ) {
        let cx = cx.add_empty_window();

        let state = ListState::new(10, crate::ListAlignment::Top, px(0.));
        state.set_size_hints(0..10, std::iter::repeat_n(size(px(100.), px(50.)), 10));

        struct TestView(ListState);
        impl Render for TestView {
            fn render(
                &mut self,
                _: &mut crate::BuildCx<'_>,
                _: &mut Context<Self>,
            ) -> impl IntoElement {
                list(self.0.clone(), |_, _, _| {
                    div().h(px(50.)).w_full().into_any()
                })
                .w_full()
                .h_full()
            }
        }

        let view = cx.update(|_, cx| cx.new(|_| TestView(state.clone())));

        state.set_follow_mode(FollowMode::Tail);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });
        assert!(state.is_following_tail());

        state.scrollbar_drag_started();

        state.splice(10..10, 10);
        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.clone().into_any_element()
        });

        state.set_offset_from_scrollbar(point(px(0.), px(-300.)));
        state.scrollbar_drag_ended();

        cx.draw(point(px(0.), px(0.)), size(px(100.), px(200.)), |_, _| {
            view.into_any_element()
        });

        assert!(
            state.is_following_tail(),
            "follow_tail should re-engage when the user drags the scrollbar to \
             the bottom of its track, even when content has grown during the drag \
             (so frozen_bottom < live_bottom)"
        );
    }
}
