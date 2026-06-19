//! jjscratch — a native, GPU-rendered Jujutsu client.
//!
//! UI is a close copy of `lightjj` (a Svelte web client for jj), rendered with
//! Vello on wgpu. Backend reads jj repositories in-process via `jj-lib`.
//!
//! Module map (more modules land here as the parallel build progresses):
//! - [`render`] — headless Vello -> PNG pipeline (the dev/screenshot loop).
//! - [`window`] — windowed Vello-on-wgpu surface render path (interactive bin).

pub mod graph_layout;
pub mod input;
pub mod model;
pub mod render;
pub mod text;
pub mod theme;
pub mod ui;
pub mod watch;
pub mod window;

pub use render::{Headless, Image};
