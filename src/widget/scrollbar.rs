// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License in the LICENSE-APACHE file or at:
//     https://www.apache.org/licenses/LICENSE-2.0

//! `ScrollBar` control

use std::fmt::Debug;

use crate::event::{self, Address, Event, Handler, Manager, PressSource, Response};
use crate::layout::{AxisInfo, Direction, SizeRules};
use crate::macros::Widget;
use crate::theme::{DrawHandle, SizeHandle};
use crate::{CoreData, TkWindow, Widget, WidgetCore};
use kas::geom::Rect;

/// A scroll bar
///
/// Scroll bars allow user-input of a value between 0 and a defined maximum.
#[widget]
#[derive(Clone, Debug, Default, Widget)]
pub struct ScrollBar<D: Direction> {
    #[core]
    core: CoreData,
    direction: D,
    // Terminology assumes vertical orientation:
    width: u32,
    min_handle_len: u32,
    handle_len: u32,
    page_length: u32, // contract: > 0
    max_value: u32,
    value: u32,
    press_source: Option<PressSource>,
    press_offset: i32,
}

impl<D: Direction + Default> ScrollBar<D> {
    /// Construct a scroll bar
    ///
    /// Default values are assumed for all parameters.
    pub fn new() -> Self {
        ScrollBar::new_with_direction(D::default())
    }
}

impl<D: Direction> ScrollBar<D> {
    /// Construct a scroll bar with the given direction
    ///
    /// Default values are assumed for all parameters.
    #[inline]
    pub fn new_with_direction(direction: D) -> Self {
        ScrollBar {
            core: Default::default(),
            direction,
            width: 0,
            min_handle_len: 0,
            handle_len: 0,
            page_length: 1,
            max_value: 0,
            value: 0,
            press_source: None,
            press_offset: 0,
        }
    }

    /// Set the page length
    ///
    /// See [`ScrollBar::set_lengths`].
    #[inline]
    pub fn with_lengths(mut self, page_length: u32, page_visible: u32) -> Self {
        self.set_lengths(page_length, page_visible);
        self
    }

    /// Set the page length
    ///
    /// These values control both the visible length of the scroll bar and the
    /// maximum position possible.
    ///
    /// Scroll bars are optimised for navigating a page of length `page_length`
    /// where a portion, `page_visible`, is visible. The value (page position)
    /// is rectricted to the range `[0, page_length - page_visible]`
    /// (inclusive). The length of the scroll bar is proportional to
    /// `page_visible / page_length` (with a minimum value).
    ///
    /// Any units may be used (e.g. pixels or lines).
    /// If `page_visible > page_length` then it is clamped to the latter value.
    pub fn set_lengths(&mut self, page_length: u32, page_visible: u32) {
        assert!(page_length > 0);
        self.page_length = page_length;
        self.max_value = page_length.saturating_sub(page_visible);
        self.value = self.value.min(self.max_value);
        self.update_handle();
    }

    /// Get the current value
    #[inline]
    pub fn value(&self) -> u32 {
        self.value
    }

    /// Set the value
    pub fn set_value(&mut self, tk: &mut dyn TkWindow, value: u32) {
        let value = value.min(self.max_value);
        if value != self.value {
            self.value = value;
            tk.redraw(self.id());
        }
    }

    #[inline]
    fn len(&self) -> u32 {
        match self.direction.is_vertical() {
            false => self.core.rect.size.0,
            true => self.core.rect.size.1,
        }
    }

    fn update_handle(&mut self) {
        let len = self.len();
        let page_len = self.page_length;
        let page_vis = page_len - self.max_value;

        let handle_len = page_vis as u64 * len as u64 / page_len as u64;
        self.handle_len = (handle_len as u32).max(self.min_handle_len).min(len);
        self.value = self.value.min(self.max_value);
    }

    // translate value to position in local coordinates
    fn position(&self) -> u32 {
        let len = self.len() - self.handle_len;
        let lhs = self.value as u64 * len as u64;
        let rhs = self.max_value as u64;
        if rhs == 0 {
            return 0;
        }
        let pos = ((lhs + (rhs / 2)) / rhs) as u32;
        pos.min(len)
    }

    // true if not equal to old value
    fn set_position(&mut self, tk: &mut dyn TkWindow, position: u32) -> bool {
        let len = self.len() - self.handle_len;
        let lhs = position as u64 * self.max_value as u64;
        let rhs = len as u64;
        if rhs == 0 {
            debug_assert_eq!(self.value, 0);
            return false;
        }
        let value = ((lhs + (rhs / 2)) / rhs) as u32;
        let value = value.min(self.max_value);
        if value != self.value {
            self.value = value;
            tk.redraw(self.id());
            return true;
        }
        false
    }
}

impl<D: Direction> Widget for ScrollBar<D> {
    fn size_rules(&mut self, size_handle: &mut dyn SizeHandle, axis: AxisInfo) -> SizeRules {
        let (thickness, _, min_len) = size_handle.scrollbar();
        if self.direction.is_vertical() == axis.vertical() {
            SizeRules::fixed(min_len)
        } else {
            SizeRules::fixed(thickness)
        }
    }

    fn set_rect(&mut self, size_handle: &mut dyn SizeHandle, rect: Rect) {
        let (thickness, min_handle_len, _) = size_handle.scrollbar();
        self.width = thickness;
        self.min_handle_len = min_handle_len;
        self.core.rect = rect;
        self.update_handle();
    }

    fn draw(&self, draw_handle: &mut dyn DrawHandle, ev_mgr: &event::Manager) {
        let dir = self.direction.is_vertical();
        let hl = ev_mgr.highlight_state(self.id());
        draw_handle.scrollbar(self.core.rect, dir, self.handle_len, self.position(), hl);
    }
}

impl<D: Direction> Handler for ScrollBar<D> {
    type Msg = u32;

    fn handle(&mut self, tk: &mut dyn TkWindow, _: Address, event: Event) -> Response<Self::Msg> {
        match event {
            Event::PressStart { source, coord, .. } => {
                // Interacting with a scrollbar with multiple presses
                // does not make sense. Any other gets aborted.
                // TODO: only if request_press_grab succeeds
                self.press_source = Some(source);
                tk.update_data(&mut |data| data.request_press_grab(source, self, coord));

                // Event delivery implies coord is over the scrollbar.
                let (pointer, offset) = match self.direction.is_vertical() {
                    false => (coord.0, self.core.rect.pos.0),
                    true => (coord.1, self.core.rect.pos.1),
                };
                let position = self.position() as i32;
                let h_start = offset + position;

                if pointer >= h_start && pointer < h_start + self.handle_len as i32 {
                    // coord is on the scroll handle
                    self.press_offset = position - pointer;
                    Response::None
                } else {
                    // coord is not on the handle; we move the bar immediately
                    self.press_offset = -offset - (self.handle_len / 2) as i32;
                    let position = (pointer + self.press_offset).max(0) as u32;
                    let moved = self.set_position(tk, position);
                    debug_assert!(moved);
                    tk.redraw(self.id());
                    Response::Msg(self.value)
                }
            }
            Event::PressMove { source, coord, .. } if Some(source) == self.press_source => {
                let pointer = match self.direction.is_vertical() {
                    false => coord.0,
                    true => coord.1,
                };
                let position = (pointer + self.press_offset).max(0) as u32;
                if self.set_position(tk, position) {
                    tk.redraw(self.id());
                    Response::Msg(self.value)
                } else {
                    Response::None
                }
            }
            Event::PressEnd { source, .. } if Some(source) == self.press_source => {
                self.press_source = None;
                Response::None
            }
            e @ _ => Manager::handle_generic(self, tk, e),
        }
    }
}
