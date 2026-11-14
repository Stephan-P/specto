//! The UI-agnostic viewer core shared by the desktop and Android apps.
//!
//! [`Viewer`] owns layout (single page, continuous, two-page), zoom (fixed,
//! fit width, fit page), scrolling and hit-testing, and plans each [`Frame`]:
//! which cached tiles to draw where (in device pixels, snapped so tiles at the
//! exact device scale are drawn 1:1) and which tiles to request from the
//! `pdfrs_render::scheduler`, in priority order.
//!
//! The frontend's job is small: forward input, keep a tile cache, and draw
//! rectangles. It never does layout maths.

mod layout;
mod viewer;

pub use layout::{LayoutMode, group_spreads};
pub use viewer::{
    Frame, PageDraw, Settings, TileDraw, ViewPage, ViewState, Viewer, ZoomMode, placeholder_level,
};

#[cfg(test)]
mod tests;
