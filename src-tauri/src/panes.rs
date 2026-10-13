//! The window's panes: the planner on the left and, when shown, the terminal on the right.

use std::{path::PathBuf, sync::Mutex};

use tauri::{PhysicalPosition, PhysicalSize, Position, Rect, Size, Webview, Window};

use crate::config;

const MIN_TERMINAL_WIDTH: u32 = 260;
const MIN_PLANNER_WIDTH: u32 = 480;

#[derive(Clone, Copy)]
struct Split {
    visible: bool,
    /// Terminal width in physical pixels; 0 until the user has shown it once.
    width: u32,
}

pub struct Panes {
    window: Window,
    planner: Webview,
    terminal: Webview,
    split: Mutex<Split>,
    /// Where the split is remembered between runs.
    data_dir: PathBuf,
}

fn rect(x: u32, width: u32, height: u32) -> Rect {
    Rect {
        position: Position::Physical(PhysicalPosition::new(x as i32, 0)),
        size: Size::Physical(PhysicalSize::new(width, height)),
    }
}

impl Panes {
    pub fn new(
        window: Window,
        planner: Webview,
        terminal: Webview,
        visible: bool,
        width: u32,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            window,
            planner,
            terminal,
            split: Mutex::new(Split { visible, width }),
            data_dir,
        }
    }

    /// Fit both webviews to the window. Call after anything that changes its size.
    pub fn layout(&self) {
        let Ok(size) = self.window.inner_size() else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return; // minimised
        }
        let mut split = self.split.lock().unwrap();
        if !split.visible {
            let _ = self.terminal.hide();
            let _ = self.planner.set_bounds(rect(0, size.width, size.height));
            return;
        }
        if split.width == 0 {
            split.width = size.width * 2 / 5;
        }
        let widest = size.width.saturating_sub(MIN_PLANNER_WIDTH).max(MIN_TERMINAL_WIDTH);
        split.width = split.width.clamp(MIN_TERMINAL_WIDTH, widest);
        let planner_width = size.width.saturating_sub(split.width);
        let _ = self.planner.set_bounds(rect(0, planner_width, size.height));
        let _ = self.terminal.set_bounds(rect(planner_width, split.width, size.height));
        let _ = self.terminal.show();
    }

    pub fn visible(&self) -> bool {
        self.split.lock().unwrap().visible
    }

    pub fn set_visible(&self, visible: bool) {
        self.split.lock().unwrap().visible = visible;
        self.layout();
        if visible {
            let _ = self.terminal.set_focus();
        }
        self.remember();
    }

    /// Move the terminal's left edge by `delta` physical pixels (negative widens it).
    pub fn drag(&self, delta: f64) {
        {
            let mut split = self.split.lock().unwrap();
            split.width = (split.width as f64 - delta).round().max(0.0) as u32;
        }
        self.layout();
    }

    /// Save the split so the next run starts the same way.
    pub fn remember(&self) {
        let split = *self.split.lock().unwrap();
        config::save_terminal(&self.data_dir, split.visible, split.width);
    }
}
