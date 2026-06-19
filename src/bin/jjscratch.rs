//! `jjscratch` — the interactive, on-screen Jujutsu client.
//!
//! This is the REAL windowed app: a [`winit`] window with a Vello-on-wgpu
//! surface render loop (see [`jjscratch::window::WindowRenderer`]), keyboard
//! navigation routed through the shared [`jjscratch::input`] keymap, and a live
//! `.jj/` filesystem watch (via [`jjscratch::watch`]) so the window re-renders
//! the moment the repo changes on disk.
//!
//! ```text
//! cargo run --bin jjscratch --features jjlib -- -R /path/to/repo
//! ```
//!
//! Controls and the `VK_ICD_FILENAMES` caveat are documented in
//! `docs/RUNNING.md`. A headless `--smoke` flag does one offscreen render (no
//! window) to prove the build/data wiring on a display-less machine.
//!
//! Built with `required-features = ["jjlib"]`: the real `main` lives behind the
//! `jjlib` cfg, and a stub `main` below prints a clear error if someone forces a
//! build without the feature.

// ------------------------------------------------------------------------
// Without the jj-lib backend there is no repo to load, so the app can't run.
// `required-features` makes `cargo run --bin jjscratch` (no `--features jjlib`)
// skip the bin entirely; this stub only fires if someone builds it explicitly
// with the feature off, and explains how to fix it.
#[cfg(not(feature = "jjlib"))]
fn main() {
    eprintln!(
        "jjscratch: this binary requires the `jjlib` feature (the in-process \
         jj-lib backend).\n\
         Rebuild with:  cargo run --bin jjscratch --features jjlib -- -R <repo>"
    );
    std::process::exit(2);
}

#[cfg(feature = "jjlib")]
fn main() -> anyhow::Result<()> {
    real::main()
}

#[cfg(feature = "jjlib")]
mod real {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::{Context, Result};
    use winit::application::ApplicationHandler;
    use winit::dpi::{LogicalSize, PhysicalPosition};
    use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
    use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
    use winit::keyboard::{Key, ModifiersState, NamedKey};
    use winit::window::{Window, WindowId};

    use jjscratch::input::{self, DragState, MouseEvent, MouseOutcome};
    use jjscratch::model::jjlib;
    use jjscratch::model::{CommitDiff, Snapshot};
    use jjscratch::text::Fonts;
    use jjscratch::ui::{self, Frame, FrameLayout, UiState};
    use jjscratch::watch::{ReactiveReloader, Watcher};
    use jjscratch::window::WindowRenderer;
    use vello::Scene;

    /// User event posted from the watcher thread to wake the event loop when the
    /// repo changes on disk (so the loop is never blocked polling for ticks).
    #[derive(Debug, Clone, Copy)]
    enum AppEvent {
        /// The `.jj/` watcher observed a coalesced change; reload + redraw.
        RepoChanged,
    }

    /// Parsed command line: `jjscratch [-R <repo>] [--smoke]`.
    struct Cli {
        repo: PathBuf,
        smoke: bool,
    }

    fn parse_cli() -> Result<Cli> {
        let mut repo: Option<PathBuf> = None;
        let mut smoke = false;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-R" | "--repo" => {
                    repo = Some(PathBuf::from(
                        args.next().context("-R requires a path argument")?,
                    ));
                }
                "--smoke" => smoke = true,
                "-h" | "--help" => {
                    println!(
                        "jjscratch — interactive Jujutsu client\n\n\
                         USAGE:\n  jjscratch [-R <repo>] [--smoke]\n\n\
                         OPTIONS:\n\
                         \x20 -R, --repo <path>   repo to open (default: current dir)\n\
                         \x20 --smoke             one headless offscreen render, then exit\n\
                         \x20 -h, --help          show this help\n\n\
                         CONTROLS: j/k or arrows = move, 1/2/3 = views, 4/5 = oplog/evolog,\n\
                         \x20         t = theme, Ctrl+K = command palette, Esc = close palette.\n\
                         The window live-updates as the repo's .jj/ changes on disk."
                    );
                    std::process::exit(0);
                }
                other if other.starts_with("-R") && other.len() > 2 => {
                    // `-R/path` (attached form).
                    repo = Some(PathBuf::from(&other[2..]));
                }
                other => {
                    anyhow::bail!("unexpected argument: {other:?} (try --help)");
                }
            }
        }
        let repo = match repo {
            Some(p) => p,
            None => std::env::current_dir().context("resolving current dir")?,
        };
        Ok(Cli { repo, smoke })
    }

    pub fn main() -> Result<()> {
        let cli = parse_cli()?;

        // The repo's data layer: reactive reloader (open repo + snapshot + diff
        // cache, op-id gated) primed with the initial state. Shared by both the
        // smoke path and the window path.
        let mut data = RepoData::open(&cli.repo)?;

        if cli.smoke {
            return run_smoke(&data);
        }

        // The live `.jj/` watcher. Held by the app; a tiny forwarding thread
        // turns its blocking ticks into non-blocking `AppEvent::RepoChanged`
        // wakeups on the winit event loop.
        let watcher = Watcher::new(&cli.repo).context("starting .jj/ watcher")?;

        let event_loop = EventLoop::<AppEvent>::with_user_event()
            .build()
            .context("building winit event loop")?;
        // Reactive UI: wait for events (input / fs change) rather than busy-loop.
        event_loop.set_control_flow(ControlFlow::Wait);
        let proxy = event_loop.create_proxy();

        spawn_watch_forwarder(watcher, proxy);

        // Default state, cursor on the working copy (matches `shot`).
        let mut state = UiState::default();
        state.selected = data
            .snapshot
            .nodes
            .iter()
            .position(|n| n.is_working_copy)
            .unwrap_or(0);
        // Tell the reloader which commit's diff to materialize on reloads.
        data.select(&state);

        let mut app = App {
            data,
            state,
            fonts: Fonts::bundled(),
            window: None,
            renderer: None,
            modifiers: ModifiersState::empty(),
            drag: DragState::default(),
            cursor: PhysicalPosition::new(0.0, 0.0),
        };
        event_loop.run_app(&mut app).context("winit event loop")?;
        Ok(())
    }

    /// One headless offscreen render to prove the build/data wiring without a
    /// display. Uses the existing [`jjscratch::Headless`] PNG path.
    fn run_smoke(data: &RepoData) -> Result<()> {
        use jjscratch::Headless;
        let (w, h) = (1280u32, 800u32);
        let mut hl = Headless::new()?;
        eprintln!(
            "smoke: adapter {} ({:?}, {:?})",
            hl.adapter_info.name, hl.adapter_info.device_type, hl.adapter_info.backend
        );
        let fonts = Fonts::bundled();
        let mut state = UiState::default();
        state.selected = data
            .snapshot
            .nodes
            .iter()
            .position(|n| n.is_working_copy)
            .unwrap_or(0);
        let clear = state.theme.palette().base;
        let frame = Frame {
            oplog: &data.oplog,
            ..Default::default()
        };
        let mut scene = Scene::new();
        ui::build_scene(
            &mut scene,
            &data.snapshot,
            data.diff.as_deref(),
            &state,
            &fonts,
            &frame,
            w as f64,
            h as f64,
        );
        let img = hl.render(&scene, w, h, clear)?;
        let out = "jjscratch-smoke.png";
        img.save_png(out)?;
        eprintln!(
            "smoke: rendered {} revisions, {} diff files -> {out} ({w}x{h})",
            data.snapshot.revision_count(),
            data.diff.as_ref().map(|d| d.files.len()).unwrap_or(0),
        );
        Ok(())
    }

    /// Forward the (blocking) watcher's coalesced ticks onto the winit event
    /// loop as non-blocking user events, so the render loop never blocks on the
    /// watcher. Uses latest-wins draining: a burst of ticks wakes us once.
    fn spawn_watch_forwarder(watcher: Watcher, proxy: EventLoopProxy<AppEvent>) {
        std::thread::spawn(move || {
            // Keep `watcher` owned here so the OS watch stays alive for the
            // process lifetime; dropping it would stop the watch.
            loop {
                // Block (with a long timeout so the thread can still exit if the
                // loop closes) for the next change, then coalesce.
                let Some(_tick) = watcher.next_change_timeout(Duration::from_secs(3600)) else {
                    continue;
                };
                let _ = watcher.latest_pending();
                // If the event loop has closed, `send_event` fails — stop.
                if proxy.send_event(AppEvent::RepoChanged).is_err() {
                    return;
                }
            }
        });
    }

    // --------------------------------------------------------------------
    // Repo data layer (jjlib-backed).

    /// Holds the open repo + the current snapshot/diff/oplog the renderer needs.
    /// Wraps [`ReactiveReloader`] (op-id-gated reload + per-commit diff cache).
    struct RepoData {
        reloader: ReactiveReloader,
        snapshot: Snapshot,
        diff: Option<Arc<CommitDiff>>,
        oplog: Vec<jjlib::OpEntry>,
    }

    impl RepoData {
        fn open(repo: &PathBuf) -> Result<Self> {
            eprintln!("jjscratch: opening repo {}", repo.display());
            let mut reloader = ReactiveReloader::open(repo)
                .with_context(|| format!("opening jj repo at {}", repo.display()))?;
            // Prime the initial state (force past the op-id gate).
            let out = reloader.reload_forced().context("initial repo load")?;
            let snapshot = out.snapshot;
            let diff = out.diff;
            let oplog = jjlib::oplog(reloader.loaded(), 30).unwrap_or_default();
            eprintln!(
                "jjscratch: loaded {} revisions from {} (workspace {}), {} ops",
                snapshot.revision_count(),
                snapshot.repo_name,
                snapshot.workspace_name,
                oplog.len(),
            );
            Ok(Self {
                reloader,
                snapshot,
                diff,
                oplog,
            })
        }

        /// The commit id (hex) the cursor currently selects, if any.
        fn selected_commit_id(&self, state: &UiState) -> Option<String> {
            self.snapshot
                .nodes
                .get(state.selected)
                .map(|n| n.commit_id.clone())
        }

        /// Point the reloader's diff at the selected row's commit.
        fn select(&mut self, state: &UiState) {
            let id = self.selected_commit_id(state);
            self.reloader.select_commit(id);
        }

        /// Recompute just the selected commit's diff (used when the cursor moves
        /// but the repo did not change — so a full reload would be wasted work).
        /// Reuses the reloader's per-commit diff cache.
        fn refresh_selected_diff(&mut self, state: &UiState) {
            let Some(id) = self.selected_commit_id(state) else {
                self.diff = None;
                return;
            };
            self.reloader.select_commit(Some(id.clone()));
            match jjlib::commit_diff(self.reloader.loaded(), &id) {
                Ok(d) => self.diff = Some(Arc::new(d)),
                Err(e) => eprintln!("jjscratch: diff for {id} failed: {e:#}"),
            }
        }

        /// Reload the snapshot (+ selected diff + oplog) after a `.jj/` change.
        /// Op-id-gated: returns `true` iff something actually changed.
        fn reload_on_change(&mut self, state: &mut UiState) -> bool {
            match self.reloader.reload() {
                Ok(Some(out)) => {
                    self.snapshot = out.snapshot;
                    self.oplog = jjlib::oplog(self.reloader.loaded(), 30).unwrap_or_default();
                    // The log may have shrunk under the cursor; clamp the
                    // selection so the highlighted row stays in range. (Renderers
                    // already tolerate an out-of-range index, but keeping it
                    // valid means the diff we show follows a real row.)
                    let last = self.snapshot.nodes.len().saturating_sub(1);
                    if self.snapshot.nodes.is_empty() {
                        state.selected = 0;
                    } else if state.selected > last {
                        state.selected = last;
                    }
                    // Re-point the diff at whatever the cursor now selects. (The
                    // clamp above may have moved the cursor off the commit
                    // `out.diff` was computed for, so resolve from the fresh
                    // snapshot rather than reusing `out.diff`.)
                    self.refresh_selected_diff(state);
                    true
                }
                Ok(None) => false, // op unchanged: spurious touch, nothing to do.
                Err(e) => {
                    eprintln!("jjscratch: reload failed: {e:#}");
                    false
                }
            }
        }
    }

    // --------------------------------------------------------------------
    // winit application.

    struct App {
        data: RepoData,
        state: UiState,
        fonts: Fonts,
        window: Option<Arc<Window>>,
        renderer: Option<WindowRenderer>,
        modifiers: ModifiersState,
        /// In-progress divider drag, threaded through `input::handle_mouse`
        /// across Down/Move/Up (lightjj's transient `draggingDivider`).
        drag: DragState,
        /// Last cursor position in LOGICAL window coordinates. winit reports the
        /// cursor on `CursorMoved`, but click/wheel events carry no position, so
        /// we remember the latest move and reuse it for those.
        cursor: PhysicalPosition<f64>,
    }

    impl App {
        /// Build the scene at the window's current physical size and present it.
        fn redraw(&mut self) -> Result<()> {
            let (Some(window), Some(renderer)) = (&self.window, self.renderer.as_mut()) else {
                return Ok(());
            };
            let phys = window.inner_size();
            let scale = window.scale_factor();
            let (pw, ph) = (phys.width.max(1), phys.height.max(1));
            // Logical size the UI lays out at; the scene is then scaled by the
            // HiDPI factor so vectors/glyphs rasterize crisply at device res
            // (same trick `shot --scale` uses).
            let logical_w = pw as f64 / scale;
            let logical_h = ph as f64 / scale;

            let palette = self.state.theme.palette();
            let clear = palette.base;
            let frame = Frame {
                oplog: &self.data.oplog,
                ..Default::default()
            };

            let mut ui_scene = Scene::new();
            ui::build_scene(
                &mut ui_scene,
                &self.data.snapshot,
                self.data.diff.as_deref(),
                &self.state,
                &self.fonts,
                &frame,
                logical_w,
                logical_h,
            );

            // Append under the HiDPI scale so the surface (sized in physical px)
            // gets a crisp render. At scale 1.0 this is a cheap identity append.
            let mut scene = Scene::new();
            scene.append(&ui_scene, Some(vello::kurbo::Affine::scale(scale)));
            renderer.render(&scene, clear)
        }

        /// Translate a winit key press to the token string `input::handle_key`
        /// expects, route it, and react (reload diff on a selection change).
        /// Returns `true` if a redraw is warranted.
        fn on_key(&mut self, key: &Key, text: Option<&str>) -> bool {
            let ctrl = self.modifiers.control_key() || self.modifiers.super_key();

            // Build the token. Ctrl/Cmd+K is the only modified binding lightjj
            // (and our router) recognizes; other modified combos are ignored so
            // they don't leak into the palette query.
            let token: Option<String> = match key {
                Key::Named(named) => match named {
                    NamedKey::ArrowDown => Some("ArrowDown".into()),
                    NamedKey::ArrowUp => Some("ArrowUp".into()),
                    NamedKey::Escape => Some("Escape".into()),
                    NamedKey::Backspace => Some("Backspace".into()),
                    NamedKey::Space => Some(" ".into()),
                    _ => None, // ArrowLeft/Right, Enter, etc.: no binding (yet).
                },
                Key::Character(s) => {
                    let s = s.as_str();
                    if ctrl {
                        // Only Ctrl/Cmd+K maps to a binding; swallow other combos.
                        if s.eq_ignore_ascii_case("k") {
                            Some("ctrl+k".into())
                        } else {
                            None
                        }
                    } else if s.chars().count() == 1 {
                        // Single printable char: drives global keys (j/k/1-5/t)
                        // when the palette is closed, or query text when open.
                        Some(s.to_string())
                    } else {
                        None
                    }
                }
                _ => None,
            };

            // Fall back to `text` for any printable the logical key didn't yield
            // (e.g. shifted symbols) while the palette is capturing input.
            let token = token.or_else(|| {
                if !ctrl && self.state.palette_open {
                    text.filter(|t| t.chars().count() == 1 && !t.chars().next().unwrap().is_control())
                        .map(|t| t.to_string())
                } else {
                    None
                }
            });

            let Some(token) = token else {
                return false;
            };

            let selection_changed = input::handle_key(&token, &mut self.state, &self.data.snapshot);
            if selection_changed {
                // Cursor moved to a different revision: reload that commit's diff
                // (repo unchanged, so no full reload).
                self.data.refresh_selected_diff(&self.state);
            }
            // Any handled key may have changed the UI (view switch, theme,
            // drawer, palette query), so always request a redraw.
            true
        }

        /// The frame layout at the window's current LOGICAL size — the same one
        /// `redraw` lays the scene out at (and `build_scene` recomputes), so a
        /// pointer hit-test maps to exactly the geometry on screen. Returns
        /// `None` before the window exists.
        fn layout(&self) -> Option<FrameLayout> {
            let window = self.window.as_ref()?;
            let phys = window.inner_size();
            let scale = window.scale_factor();
            let logical_w = phys.width.max(1) as f64 / scale;
            let logical_h = phys.height.max(1) as f64 / scale;
            let drawer_open = self.state.oplog_open || self.state.evolog_open;
            Some(FrameLayout::compute(
                logical_w,
                logical_h,
                self.state.panel_width,
                self.state.active_view,
                drawer_open,
            ))
        }

        /// Route a translated pointer event through `input::handle_mouse` and
        /// react. Returns `true` if a redraw is warranted (the caller requests
        /// it). On a selection change we reload the selected commit's diff via
        /// the same path the keyboard navigation uses.
        fn on_mouse(&mut self, ev: MouseEvent) -> bool {
            let Some(layout) = self.layout() else {
                return false;
            };
            let outcome = input::handle_mouse(
                ev,
                &mut self.state,
                &self.data.snapshot,
                &layout,
                &mut self.drag,
                &self.fonts,
            );
            match outcome {
                MouseOutcome::SelectionChanged => {
                    self.data.refresh_selected_diff(&self.state);
                    true
                }
                MouseOutcome::Redraw => true,
                MouseOutcome::None => false,
            }
        }

        /// Current cursor position in LOGICAL window coordinates (physical px
        /// divided by the HiDPI scale factor), matching the logical layout the
        /// UI lays out at. winit reports cursor positions in physical px.
        fn cursor_logical(&self) -> (f64, f64) {
            let scale = self
                .window
                .as_ref()
                .map(|w| w.scale_factor())
                .unwrap_or(1.0);
            (self.cursor.x / scale, self.cursor.y / scale)
        }
    }

    impl ApplicationHandler<AppEvent> for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.window.is_some() {
                return; // Already created (e.g. resumed after suspend).
            }
            let attrs = Window::default_attributes()
                .with_title("jjscratch")
                .with_inner_size(LogicalSize::new(1280.0, 800.0));
            let window = match event_loop.create_window(attrs) {
                Ok(w) => Arc::new(w),
                Err(e) => {
                    eprintln!("jjscratch: failed to create window: {e}");
                    event_loop.exit();
                    return;
                }
            };
            let size = window.inner_size();
            match WindowRenderer::new(window.clone(), size.width, size.height) {
                Ok(r) => {
                    let info = r.adapter_info();
                    eprintln!(
                        "jjscratch: GPU adapter {} ({:?}, {:?})",
                        info.name, info.device_type, info.backend
                    );
                    self.renderer = Some(r);
                }
                Err(e) => {
                    eprintln!("jjscratch: failed to init GPU surface: {e:#}");
                    event_loop.exit();
                    return;
                }
            }
            self.window = Some(window);
        }

        fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: AppEvent) {
            match event {
                AppEvent::RepoChanged => {
                    if self.data.reload_on_change(&mut self.state) {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
            }
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _id: WindowId,
            event: WindowEvent,
        ) {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),

                WindowEvent::ModifiersChanged(mods) => {
                    self.modifiers = mods.state();
                }

                WindowEvent::KeyboardInput { event, .. } => {
                    // Only act on presses (and auto-repeats): release events
                    // don't drive navigation here.
                    if event.state == ElementState::Pressed {
                        let text = event.text.as_ref().map(|s| s.as_str());
                        if self.on_key(&event.logical_key, text) {
                            if let Some(w) = &self.window {
                                w.request_redraw();
                            }
                        }
                    }
                }

                // Mouse: translate winit pointer events into the shared
                // `input::MouseEvent` (logical coords) and route them through
                // `input::handle_mouse`, the same pure router the cross-driver
                // harness uses. winit reports positions in PHYSICAL px, so we
                // convert to logical (÷ scale_factor) to match the logical size
                // the UI lays out at (see `redraw`/`cursor_logical`).
                WindowEvent::CursorMoved { position, .. } => {
                    self.cursor = position;
                    let (x, y) = self.cursor_logical();
                    if self.on_mouse(MouseEvent::Move { x, y }) {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }

                WindowEvent::MouseInput { state, button, .. } => {
                    // Map winit's button enum to the router's 0=Left/1=Middle/
                    // 2=Right convention; ignore extra/back/forward buttons.
                    let btn = match button {
                        MouseButton::Left => Some(0u8),
                        MouseButton::Middle => Some(1u8),
                        MouseButton::Right => Some(2u8),
                        _ => None,
                    };
                    let (x, y) = self.cursor_logical();
                    let ev = match state {
                        // Up carries no button (it only ends a divider drag).
                        ElementState::Released => Some(MouseEvent::Up { x, y }),
                        ElementState::Pressed => btn.map(|b| MouseEvent::Down { x, y, button: b }),
                    };
                    if let Some(ev) = ev {
                        if self.on_mouse(ev) {
                            if let Some(w) = &self.window {
                                w.request_redraw();
                            }
                        }
                    }
                }

                WindowEvent::MouseWheel { delta, .. } => {
                    // Normalize both wheel encodings to a logical-pixel scroll
                    // step (down = positive, matching `MouseEvent::Wheel`).
                    // LineDelta is in text lines; scale by a sensible per-line
                    // step. PixelDelta (precise trackpads) is physical px → ÷
                    // scale to logical. winit's sign is up = positive, so negate
                    // to make scrolling down advance the content.
                    const LINE_STEP: f64 = 40.0;
                    let delta_y = match delta {
                        MouseScrollDelta::LineDelta(_, lines) => -(lines as f64) * LINE_STEP,
                        MouseScrollDelta::PixelDelta(pos) => {
                            let scale = self
                                .window
                                .as_ref()
                                .map(|w| w.scale_factor())
                                .unwrap_or(1.0);
                            -pos.y / scale
                        }
                    };
                    let (x, y) = self.cursor_logical();
                    if self.on_mouse(MouseEvent::Wheel { x, y, delta_y }) {
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }

                WindowEvent::Resized(size) => {
                    if let Some(r) = self.renderer.as_mut() {
                        r.resize(size.width, size.height);
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }

                WindowEvent::ScaleFactorChanged { .. } => {
                    // The new scale factor is read live in `redraw` via
                    // `window.scale_factor()`; a resize event usually follows,
                    // but request a redraw to be safe.
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }

                WindowEvent::RedrawRequested => {
                    if let Err(e) = self.redraw() {
                        eprintln!("jjscratch: render error: {e:#}");
                    }
                }

                _ => {}
            }
        }
    }
}
