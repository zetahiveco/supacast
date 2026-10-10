//! eframe application: owns shared state, pumps worker events, and drives
//! the three viewports (launcher, settings, dictate ring).

use chrono::Local;
use std::sync::mpsc::{channel, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::ai::{self, StreamEvent};
use crate::audio;
use crate::clipboard_hist::{self, ClipEntry};
use crate::dictations::{self, Dictation};
use crate::events::{DictateMode, DPhase, DictateState, EventTx, UiEvent};
use crate::hotkeys::{self, HotkeyEvent};
use crate::notes::{self, Note};
#[cfg(target_os = "macos")]
use crate::paste_focus;
use crate::search::{self, SearchResult};
use crate::settings::{self, Settings};
use crate::todos::{self, Todo};
use crate::ui;

/// Sizes (logical pixels) of the three windows — matching tauri.conf.json.
pub const LAUNCHER_SIZE: (f32, f32) = (720.0, 480.0);
pub const SETTINGS_SIZE: (f32, f32) = (560.0, 460.0);
pub const RING_SIZE: (f32, f32) = (280.0, 240.0);

const HOLD_MS: u64 = 300; // hold Enter this long before the ring records
const MAX_RECORD_MS: u64 = 60_000; // safety cap while holding Enter
const SEARCH_DEBOUNCE_MS: u64 = 150;

// ---------------------------------------------------------------------------
// Shared state (main thread + worker threads)
// ---------------------------------------------------------------------------

pub struct Shared {
    /// egui context — workers call `request_repaint()` after sending events.
    pub ctx: OnceLock<egui::Context>,
    pub events_tx: EventTx,
    /// Global hotkey manager. macOS delivers Carbon hotkey events through
    /// the main-thread event loop, so the manager is created lazily on the
    /// first `logic()` call — i.e. once eframe's winit loop is running.
    pub hotkeys: Mutex<Option<hotkeys::Hotkeys>>,
    pub settings: Mutex<Settings>,
    pub dictate: Mutex<DictateState>,
    pub recorder: Mutex<Option<audio::Recorder>>,
    /// Conversation history of the current dictate (Supacast) thread —
    /// read by the dictate worker so follow-up dictations keep context.
    pub dictate_history: Mutex<Vec<ai::ChatMsg>>,
    /// Whether the physical (global) Enter key is currently held.
    pub enter_held: AtomicBool,
}

impl Shared {
    pub fn repaint(&self) {
        if let Some(ctx) = self.ctx.get() {
            ctx.request_repaint();
        }
    }

    /// (Re)register the launcher toggle shortcut (no-op until the manager
    /// is created on the main thread).
    pub fn set_toggle(&self, raw: &str) {
        if let Some(hk) = self.hotkeys.lock().unwrap().as_mut() {
            hk.set_toggle(raw);
        }
    }

    /// Toggle the global Enter capture for dictation.
    pub fn set_enter_capture(&self, enabled: bool) {
        if let Some(hk) = self.hotkeys.lock().unwrap().as_mut() {
            hk.set_enter_capture(enabled);
        }
    }

    /// Create the hotkey manager on the first main-thread call — must run
    /// once eframe's event loop is up (macOS delivers via the main run loop).
    fn ensure_hotkeys(&self) {
        let mut hk = self.hotkeys.lock().unwrap();
        if hk.is_none() {
            let mut h = hotkeys::Hotkeys::new();
            let raw = self.settings.lock().unwrap().shortcut.clone();
            h.set_toggle(&raw);
            eprintln!("Supacast: registered launcher shortcut '{raw}'");
            *hk = Some(h);
        }
    }
}

// ---------------------------------------------------------------------------
// Launcher UI state (main-thread only)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum View {
    Search,
    Todos,
    Notes { query: String },
    Clipboard,
    Dictations,
    Chat,
}

#[derive(Debug, Clone, Default)]
pub struct ChatTurn {
    pub role: String, // "user" | "assistant"
    pub content: String,
    pub thinking: String,
    pub tools: Vec<String>,
    pub streaming: bool,
    pub error: bool,
    /// True until the first stream event arrives.
    pub waiting: bool,
    pub started_at: Option<Instant>,
}

#[derive(PartialEq)]
pub enum SettingsStatus {
    Idle,
    Saved,
    Error(String),
}

/// What the single root window is currently showing. The original Tauri app
/// had three windows; here one window morphs between them (egui viewports
/// can't be driven while the root is hidden, but viewport *commands* issued
/// from `logic()` still apply, so mode switches work from worker events).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowMode {
    Hidden,
    Launcher,
    Settings,
    Dictate,
}

pub struct App {
    pub shared: Arc<Shared>,
    events_rx: Receiver<UiEvent>,
    _tray: tray_icon::TrayIcon, // keep alive

    // Window state.
    pub launcher_visible: bool,
    pub need_focus: bool,
    pub settings_open: bool,
    /// Which UI the root window is currently drawn as (drives window-size /
    /// position / passthrough transitions).
    window_mode: WindowMode,
    /// Whether the launcher window has actually taken keyboard focus since
    /// it was last shown. Blur-to-hide only fires after a real focus, so a
    /// window that appears while the OS is still activating the app doesn't
    /// get hidden instantly.
    launcher_had_focus: bool,
    // TEMP DEBUG fields (remove once the hotkey/visibility issues are solved).
    debug_last_visible: Option<bool>,
    debug_log_throttle: u32,
    /// Whether the last vibrancy (frosted-glass) install attempt failed —
    /// used to log retryable failures only once.
    glass_warned: bool,
    /// True after the first `logic()` call (used to apply the initial
    /// window mode once the event loop is running).
    started: bool,
    /// A window-mode transition that could not be applied yet because the
    /// monitor size was unknown (retried on the next logic tick).
    pending_mode_apply: bool,
    /// Primary monitor logical size, cached from winit each frame so
    /// windows can be centered (Spotlight style).
    pub monitor_size: Option<egui::Vec2>,

    // Launcher UI state.
    pub query: String,
    pub view: View,
    pub results: Vec<SearchResult>,
    pub active: usize,
    pub searching: bool,
    search_gen: u64,
    query_changed_at: Option<Instant>,
    /// Query queued for a debounced search; dispatched exactly once, so an
    /// empty result set can't re-trigger the search in a loop.
    pending_search: Option<String>,
    last_frame_query: String,
    ai_switch_at: Option<Instant>,
    pub todos: Vec<Todo>,
    pub notes: Vec<Note>,
    pub clips: Vec<ClipEntry>,
    pub dictations: Vec<Dictation>,
    pub sel_note: Option<String>,
    pub sel_dict: Option<String>,
    pub chat: Vec<ChatTurn>,
    pub chat_busy: bool,
    /// True while the chat thread originated from a dictate hand-off;
    /// empty-Enter then re-enters dictation mode.
    pub dictate_thread: bool,
    pub mouse_moved: bool,
    /// Scroll the keyboard-selected row into view on the next frame.
    pub scroll_pending: bool,

    // Settings window state.
    pub set_shortcut: String,
    pub set_api_key: String,
    pub set_base_url: String,
    pub set_model: String,
    pub shortcut_recording: bool,
    pub settings_status: SettingsStatus,
    /// Window height the settings view last asked for (auto-size to fit the
    /// content, no scrolling). 0 = nothing sent yet.
    pub settings_window_h: f32,
    /// Set once the user chose Quit — the OS close request is then a real
    /// quit instead of "close the settings panel".
    pub quitting: bool,

    // Dictation playback (native, replaces the <audio> element). One handle
    // per dictation so a paused dictation keeps its position and resumes
    // where it was instead of restarting from the beginning.
    pub playbacks: Vec<(String, audio::PlaybackHandle)>,
}

impl eframe::App for App {
    /// Fully transparent framebuffer in every mode. The frosted look is
    /// produced entirely by the NSVisualEffectView behind the window
    /// (real backdrop blur) with the translucent `GLASS_*` frame fills on
    /// top of it. The previous 70%-opaque `rgba(12,12,12,180)` clear color
    /// compounded with those fills (launcher ≈84% opaque, settings ≈91%),
    /// smothering the blur; with a transparent clear the fills alone tint
    /// the vibrancy, so the backdrop blur actually shows through.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    /// Runs even while the window is hidden — pump worker events + timers
    /// here. Viewport commands sent from here (showing the launcher/ring)
    /// are applied by eframe even when no ui pass runs.
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // The settings window uses the OS title bar (native close button).
        // A close request there means "close settings", not "quit" — save
        // the edits, cancel the close, and hide the panel instead.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting && self.settings_open {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.save_settings();
            self.close_settings(ctx);
        }

        // TEMP DEBUG: track real window visibility transitions.
        if let Some(win) = frame.winit_window() {
            let vis = win.is_visible();
            if vis != self.debug_last_visible {
                self.debug_last_visible = vis;
                eprintln!("[debug] window visibility -> {vis:?}");
            }
        }
        self.debug_log_throttle = self.debug_log_throttle.saturating_add(1);

        // Cache the monitor size so windows can be centered even on the
        // first frame after waking from hidden.
        if let Some(win) = frame.winit_window() {
            if let Some(monitor) = win.current_monitor().or_else(|| win.primary_monitor()) {
                let scale = monitor.scale_factor();
                let size = monitor.size();
                self.monitor_size = Some(egui::vec2(
                    (size.width as f64 / scale) as f32,
                    (size.height as f64 / scale) as f32,
                ));
            }
        }

        // First tick: apply the initial window mode now that the event loop
        // is running (the launcher opens by default so its state is
        // immediately visible/verifiable).
        if !self.started {
            self.started = true;
            self.apply_window_mode(ctx);
        }
        // Retry any window-mode transition that was deferred until the
        // monitor size became available.
        if self.pending_mode_apply {
            self.pending_mode_apply = false;
            self.apply_window_mode(ctx);
        }
        // Keep the frosted-glass layer installed. Toggling decorations
        // (the settings window uses the native title bar) can rebuild the
        // theme frame and drop the blur view — this re-installs it. Cheap:
        // apply_vibrancy returns immediately while the blur is still live.
        self.apply_glass(frame);
        // eframe force-shows the root window once after the first painted
        // frame; when hidden, actively re-hide and keep polling until it
        // sticks so no stray always-on-top blank window appears.
        if self.current_mode() == WindowMode::Hidden {
            if let Some(win) = frame.winit_window() {
                if win.is_visible().unwrap_or(false) {
                    win.set_visible(false);
                }
            }
            // Heartbeat: with the window hidden there is no other repaint
            // trigger, and without a scheduled wake the egui loop sleeps
            // forever — which would starve pump_events (global hotkeys and
            // tray menu events are only polled there).
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        self.pump_events(ctx);
        self.tick_background(ctx);
    }

    /// Draw the root window, which morphs between the launcher, the settings
    /// panel and the dictate ring depending on app state.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        match self.current_mode() {
            WindowMode::Hidden => {
                // Nothing to draw; the framebuffer is cleared to transparent.
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |_| {});
            }
            WindowMode::Launcher => {
                egui::CentralPanel::default()
                    .frame(ui::launcher_frame())
                    .show(ui, |ui| ui::launcher::draw(self, ui));

                // Spotlight behaviour: clicking away hides the launcher —
                // but only after it had a real focus (avoid the show-race).
                let focused = ctx.input(|i| i.viewport().focused);
                match focused {
                    Some(true) => self.launcher_had_focus = true,
                    Some(false) => {
                        if self.launcher_had_focus {
                            self.hide_launcher(&ctx);
                        }
                    }
                    None => {}
                }
            }
            WindowMode::Settings => {
                egui::CentralPanel::default()
                    .frame(ui::settings_frame())
                    .show(ui, |ui| ui::settings::draw(self, ui));
            }
            WindowMode::Dictate => {
                // Fully transparent background — only the glass disc around
                // the ring shows (vibrancy is hidden for this mode).
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| ui::dictate::draw(self, ui));

                // The orb pulses with the mic level; keep the frame alive.
                ctx.request_repaint_after(Duration::from_millis(33));
            }
        }

        // Finished dictations drop out; the player resets to the start.
        self.playbacks.retain(|(_, h)| !h.is_done());
    }
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        shared: Arc<Shared>,
        events_rx: Receiver<UiEvent>,
        tray: tray_icon::TrayIcon,
    ) -> Self {
        shared.ctx.set(cc.egui_ctx.clone()).ok().unwrap_or(());
        // Use the OS system font (SF Pro on macOS) for all UI text, with
        // egui's bundled fonts as fallbacks.
        crate::fonts::install(&cc.egui_ctx);

        // Sharp edges everywhere: zero the default widget corner radii so
        // every stock egui widget (buttons, text edits, collapsing headers,
        // checkboxes…) matches the sharp window frames.
        cc.egui_ctx.all_styles_mut(|style| {
            for w in [
                &mut style.visuals.widgets.noninteractive,
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
                &mut style.visuals.widgets.open,
            ] {
                w.corner_radius = egui::CornerRadius::ZERO;
            }
            style.visuals.window_corner_radius = egui::CornerRadius::ZERO;
            style.visuals.menu_corner_radius = egui::CornerRadius::ZERO;
        });
        let s = shared.settings.lock().unwrap().clone();
        // Start fully hidden; the launcher appears on hotkey / tray Open
        // (the window is always-on-top, so a default-open window would
        // sit on top of everything at boot).
        // `SUPACAST_DEBUG_SHOW=1` opens the launcher immediately (used by
        // tests / visual checks).
        let launcher_visible = std::env::var("SUPACAST_DEBUG_SHOW").is_ok();
        let need_focus = false;
        // `SUPACAST_DEBUG_SETTINGS=1` opens the settings panel immediately
        // (visual checks). Seed the edit fields like OpenSettings does.
        let debug_settings = std::env::var("SUPACAST_DEBUG_SETTINGS").is_ok();

        // The window starts hidden and click-through; make that explicit
        // (some platforms show freshly created windows briefly).
        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(true));

        let mut app = Self {
            shared,
            events_rx,
            _tray: tray,
            launcher_visible,
            need_focus,
            settings_open: debug_settings,
            window_mode: WindowMode::Hidden,
            launcher_had_focus: false,
            debug_last_visible: None,
            debug_log_throttle: 0,
            started: false,
            glass_warned: false,
            pending_mode_apply: false,
            monitor_size: None,
            query: String::new(),
            view: View::Search,
            results: Vec::new(),
            active: 0,
            searching: false,
            search_gen: 0,
            query_changed_at: None,
            pending_search: None,
            last_frame_query: String::new(),
            ai_switch_at: None,
            todos: Vec::new(),
            notes: Vec::new(),
            clips: Vec::new(),
            dictations: Vec::new(),
            sel_note: None,
            sel_dict: None,
            chat: Vec::new(),
            chat_busy: false,
            dictate_thread: false,
            mouse_moved: false,
            scroll_pending: false,
            set_shortcut: s.shortcut.clone(),
            set_api_key: s.openai_api_key.clone(),
            set_base_url: s.chat_base_url.clone(),
            set_model: s.chat_model.clone(),
            shortcut_recording: false,
            settings_status: SettingsStatus::Idle,
            settings_window_h: 0.0,
            quitting: false,
            playbacks: Vec::new(),
        };
        app.need_focus = true;
        app
    }

    // -----------------------------------------------------------------
    // Event pump + timers
    // -----------------------------------------------------------------

    fn pump_events(&mut self, ctx: &egui::Context) {
        // Create the hotkey manager lazily, once the main-thread event loop
        // is running (macOS Carbon hotkeys require this).
        self.shared.ensure_hotkeys();

        // Global hotkey presses (delivered on the main thread by the OS;
        // must be drained here, on the main thread).
        let mut hotkey_events = Vec::new();
        if let Some(hk) = self.shared.hotkeys.lock().unwrap().as_mut() {
            hk.drain_events(|ev| hotkey_events.push(ev));
        }
        for ev in hotkey_events {
            let event = match ev {
                HotkeyEvent::Toggle => UiEvent::ToggleLauncher,
                HotkeyEvent::EnterDown => UiEvent::DictateKey(true),
                HotkeyEvent::EnterUp => UiEvent::DictateKey(false),
            };
            self.handle_event(ctx, event);
        }

        // Tray menu activations (muda delivers these on the main thread).
        use tray_icon::menu::MenuEvent;
        let menu_ids: Vec<String> = MenuEvent::receiver()
            .try_iter()
            .map(|ev| ev.id().0.clone())
            .collect();
        for id in menu_ids {
            let event = match id.as_str() {
                "open" => UiEvent::ShowLauncher,
                "dictate" => UiEvent::OpenDictate(DictateMode::Supacast),
                "settings" => UiEvent::OpenSettings,
                "check-updates" => UiEvent::CheckUpdates,
                "quit" => UiEvent::Quit,
                _ => continue,
            };
            self.handle_event(ctx, event);
        }

        // Events from worker threads.
        while let Ok(event) = self.events_rx.try_recv() {
            self.handle_event(ctx, event);
        }
    }

    fn handle_event(&mut self, ctx: &egui::Context, event: UiEvent) {
        match event {
            UiEvent::ToggleLauncher => {
                eprintln!(
                    "[debug] ToggleLauncher received (launcher_visible={})",
                    self.launcher_visible
                );
                if self.launcher_visible {
                    self.hide_launcher(ctx);
                } else {
                    self.show_launcher(ctx);
                }
            }
            UiEvent::ShowLauncher => self.show_launcher(ctx),
            UiEvent::HideLauncher => self.hide_launcher(ctx),
            UiEvent::OpenSettings => {
                self.hide_launcher(ctx);
                let s = self.shared.settings.lock().unwrap().clone();
                self.set_shortcut = s.shortcut.clone();
                self.set_api_key = s.openai_api_key.clone();
                self.set_base_url = s.chat_base_url.clone();
                self.set_model = s.chat_model.clone();
                self.settings_status = SettingsStatus::Idle;
                self.settings_open = true;
                self.apply_window_mode(ctx);
            }
            UiEvent::OpenDictate(mode) => {
                self.hide_launcher(ctx);
                self.open_dictate(mode, ctx);
            }
            UiEvent::CloseDictate => self.close_dictate(ctx),
            UiEvent::DictateKey(down) => {
                if down {
                    self.dictate_key_down();
                } else {
                    self.dictate_key_up();
                }
            }
            UiEvent::StartRecording => self.start_recording(),
            UiEvent::StopRecording(cancelled) => self.stop_recording(ctx, cancelled),
            UiEvent::PasteToFocused(text) => self.paste_to_focused(&text),
            UiEvent::DictateAnswer { message, answer } => {
                self.shared.dictate.lock().unwrap().open = false;
                self.shared
                    .set_enter_capture(false);
                // Show the launcher displaying the dictated Q&A.
                let continuing = self.dictate_thread;
                self.reset_launcher();
                let user_turn = ChatTurn {
                    role: "user".into(),
                    content: message,
                    ..ChatTurn::default()
                };
                let answer_turn = ChatTurn {
                    role: "assistant".into(),
                    content: answer,
                    ..ChatTurn::default()
                };
                if continuing {
                    // Continued thread: append so the whole conversation
                    // stays visible.
                    self.chat.push(user_turn);
                    self.chat.push(answer_turn);
                } else {
                    self.chat = vec![user_turn, answer_turn];
                }
                self.dictate_thread = true;
                self.view = View::Chat;
                self.show_launcher_keep_chat(ctx);
            }
            UiEvent::ChatEvent(ev) => self.apply_chat_event(ev),
            UiEvent::SearchResults { gen, results } => {
                if gen == self.search_gen {
                    self.results = results;
                    self.active = 0;
                    self.searching = false;
                }
            }
            UiEvent::Notify { title, body } => crate::platform::notify(&title, &body),
            UiEvent::CheckUpdates => crate::platform::check_for_updates(self.shared.clone()),
            UiEvent::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn tick_background(&mut self, ctx: &egui::Context) {
        // Keep repainting while the AI is streaming so the elapsed counter
        // and caret animate.
        if self.chat_busy {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.recording_active() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
        // Animate the dictation player (progress bar) while audio plays.
        if self.playbacks.iter().any(|(_, h)| !h.is_paused()) {
            ctx.request_repaint_after(Duration::from_millis(50));
        }

        // Typing just "ai" and pausing enters AI chat mode with a fresh
        // input; any further keystroke before the timer fires cancels it.
        if let Some(t) = self.ai_switch_at {
            // Same reactive-loop caveat as the search debounce: keep the
            // loop awake so the pause timer can actually fire.
            ctx.request_repaint_after(Duration::from_millis(50));
            if t.elapsed() > Duration::from_millis(350) {
                self.ai_switch_at = None;
                if self.query.trim() == "ai" && self.view == View::Search {
                    self.query.clear();
                    self.chat.clear();
                    self.dictate_thread = false;
                    self.view = View::Chat;
                    self.need_focus = true;
                }
            }
        }

        // Detect query changes: route to command views or (after a short
        // debounce) run the app/file search. An empty input keeps the
        // current view (e.g. after adding a todo/note).
        if self.query != self.last_frame_query {
            let q = self.query.trim().to_string();
            self.last_frame_query = self.query.clone();
            self.query_changed_at = Some(Instant::now());
            self.mouse_moved = false; // new list: don't inherit stale hover
            // Invalidate any in-flight search so it can't land later.
            self.search_gen += 1;
            self.results.clear();
            self.searching = false;
            self.pending_search = None;
            if !q.is_empty() && self.view != View::Chat {
                self.route_or_search(&q);
                if self.view == View::Search {
                    // Queue exactly one debounced search for this query.
                    self.pending_search = Some(q);
                    // The debounce timer lives in this function, which only
                    // runs on frames — in eframe's reactive loop nothing
                    // else wakes us after the last keystroke, so schedule
                    // the wake or the search would never start.
                    ctx.request_repaint_after(Duration::from_millis(
                        SEARCH_DEBOUNCE_MS + 20,
                    ));
                }
            }
        } else if self.view == View::Search
            && !self.query.trim().is_empty()
            && self.results.is_empty()
            && self.pending_search.is_some()
        {
            if let Some(t) = self.query_changed_at {
                if t.elapsed() >= Duration::from_millis(SEARCH_DEBOUNCE_MS) && !self.searching {
                    // Dispatch once per query change. An empty result set
                    // must NOT re-trigger the search — the old condition
                    // ("results empty") re-spawned walkdir + mdfind in a
                    // loop forever, pinning the CPU and stuttering the UI.
                    let q = self.pending_search.take().unwrap();
                    self.dispatch_search(&q);
                }
            }
        }
    }

    /// Route the raw query to a command view; otherwise stay in search.
    fn route_or_search(&mut self, q: &str) {
        let lower = q.trim().to_lowercase();
        // "todo"/"todos" (optionally "todo tomorrow", etc.) shows the list.
        if lower == "todo"
            || lower == "todos"
            || lower.starts_with("todo ")
            || lower.starts_with("todos ")
        {
            self.view = View::Todos;
            self.load_todos();
            return;
        }
        // "notes" lists all notes; "notes wifi" filters them.
        // "note …" (singular + text) stays in search so the add-note
        // suggestion shows.
        if lower == "notes" {
            self.view = View::Notes { query: String::new() };
            self.load_notes("");
            return;
        }
        if let Some(rest) = q.trim().strip_prefix("notes ") {
            if !rest.trim().is_empty() {
                self.view = View::Notes { query: rest.to_string() };
                self.load_notes(rest);
                return;
            }
        }
        if lower.starts_with("clipboard") {
            self.view = View::Clipboard;
            self.clips = clipboard_hist::get_history();
            return;
        }
        if lower == "dictation"
            || lower == "dictations"
            || lower == "dictation history"
            || lower == "dictations history"
        {
            self.view = View::Dictations;
            self.dictations = dictations::list(None);
            return;
        }
        if lower.starts_with("dictate") {
            self.view = View::Chat; // Enter opens dictate mode
            return;
        }
        // Typing "ai" switches to chat after the pause timer (tick_background).
        if lower == "ai" {
            self.ai_switch_at = Some(Instant::now());
            self.view = View::Search;
            return;
        }
        self.ai_switch_at = None;
        self.view = View::Search;
    }

    fn dispatch_search(&mut self, q: &str) {
        self.searching = true;
        self.shared.repaint(); // show the spinner right away
        let gen = self.search_gen;
        let query = q.to_string();
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let mut results = search::search_apps(&query);
            results.extend(search::search_files(&query));
            let _ = shared.events_tx.send(UiEvent::SearchResults { gen, results });
            // Wake the UI loop so the results are processed even when the
            // window is idle-waiting for events.
            shared.repaint();
        });
    }

    // -----------------------------------------------------------------
    // Launcher window management
    // -----------------------------------------------------------------

    /// A fresh launcher session: drop the previous chat thread (it stays
    /// available under "dictations").
    fn reset_launcher(&mut self) {
        self.search_gen += 1; // cancel in-flight searches
        self.query.clear();
        self.results.clear();
        self.active = 0;
        self.view = View::Search;
        self.todos.clear();
        self.notes.clear();
        self.clips.clear();
        self.dictations.clear();
        self.sel_dict = None;
        self.chat.clear();
        self.dictate_thread = false;
        self.need_focus = true;
    }

    fn show_launcher(&mut self, ctx: &egui::Context) {
        // Opening the launcher exits dictation and closes the settings panel.
        self.close_dictate(ctx);
        self.settings_open = false;
        self.reset_launcher();
        self.show_launcher_keep_chat(ctx);
    }

    /// Show the launcher without resetting (used by the dictate hand-off).
    fn show_launcher_keep_chat(&mut self, ctx: &egui::Context) {
        self.launcher_visible = true;
        self.need_focus = true;
        self.apply_window_mode(ctx);
        ctx.request_repaint();
    }

    /// The UI the root window should currently show.
    fn current_mode(&self) -> WindowMode {
        if self.shared.dictate.lock().unwrap().open {
            WindowMode::Dictate
        } else if self.settings_open {
            WindowMode::Settings
        } else if self.launcher_visible {
            WindowMode::Launcher
        } else {
            WindowMode::Hidden
        }
    }

    /// Resize/position/show the root window for the current mode. Only
    /// sends commands on transitions (safe to call from `logic()` too —
    /// eframe applies viewport commands even without a ui pass).
    fn apply_window_mode(&mut self, ctx: &egui::Context) {
        use egui::ViewportCommand as Cmd;
        let mode = self.current_mode();
        if mode == self.window_mode {
            return;
        }
        // Positioning needs the monitor size; until winit reports it we
        // defer the whole transition so the window never shows at a wrong
        // position (e.g. top-left at startup).
        if mode != WindowMode::Hidden && self.monitor_size.is_none() {
            self.pending_mode_apply = true;
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        }
        self.pending_mode_apply = false;
        self.window_mode = mode;
        match mode {
            WindowMode::Hidden => {
                ctx.send_viewport_cmd(Cmd::MousePassthrough(true));
                ctx.send_viewport_cmd(Cmd::Visible(false));
            }
            WindowMode::Launcher => {
                // Accessory apps must be activated for the window to ever
                // become key (otherwise egui sees it as unfocused and our
                // blur-to-hide would instantly hide it again).
                crate::activate_app();
                self.launcher_had_focus = false;
                ctx.send_viewport_cmd(Cmd::MousePassthrough(false));
                ctx.send_viewport_cmd(Cmd::WindowLevel(egui::WindowLevel::AlwaysOnTop));
                ctx.send_viewport_cmd(Cmd::Decorations(false)); // custom chrome
                ctx.send_viewport_cmd(Cmd::Title("Supacast".into()));
                ctx.send_viewport_cmd(Cmd::InnerSize([LAUNCHER_SIZE.0, LAUNCHER_SIZE.1].into()));
                ctx.send_viewport_cmd(Cmd::OuterPosition(self.center_pos(LAUNCHER_SIZE)));
                ctx.send_viewport_cmd(Cmd::Visible(true));
                ctx.send_viewport_cmd(Cmd::Focus);
            }
            WindowMode::Settings => {
                // Native OS window chrome (title bar + close button).
                crate::activate_app();
                ctx.send_viewport_cmd(Cmd::MousePassthrough(false));
                ctx.send_viewport_cmd(Cmd::WindowLevel(egui::WindowLevel::AlwaysOnTop));
                ctx.send_viewport_cmd(Cmd::Decorations(true));
                ctx.send_viewport_cmd(Cmd::Title("Supacast Settings".into()));
                ctx.send_viewport_cmd(Cmd::InnerSize([SETTINGS_SIZE.0, self.settings_window_h.max(SETTINGS_SIZE.1)].into()));
                ctx.send_viewport_cmd(Cmd::OuterPosition(self.center_pos(SETTINGS_SIZE)));
                ctx.send_viewport_cmd(Cmd::Visible(true));
                ctx.send_viewport_cmd(Cmd::Focus);
            }
            WindowMode::Dictate => {
                crate::activate_app();
                ctx.send_viewport_cmd(Cmd::MousePassthrough(false));
                ctx.send_viewport_cmd(Cmd::WindowLevel(egui::WindowLevel::AlwaysOnTop));
                ctx.send_viewport_cmd(Cmd::Decorations(false));
                ctx.send_viewport_cmd(Cmd::InnerSize([RING_SIZE.0, RING_SIZE.1].into()));
                ctx.send_viewport_cmd(Cmd::OuterPosition(self.center_pos(RING_SIZE)));
                ctx.send_viewport_cmd(Cmd::Visible(true));
                ctx.send_viewport_cmd(Cmd::Focus);
            }
        }
        // The dictate ring hides the window-wide vibrancy: only its own
        // glass disc is visible there (like the React version).
        #[cfg(target_os = "macos")]
        crate::platform::macos::set_vibrancy_hidden(mode == WindowMode::Dictate);
        ctx.request_repaint();
    }

    /// Center of the primary monitor for a window of `size` logical px.
    pub fn center_pos(&self, size: (f32, f32)) -> egui::Pos2 {
        match self.monitor_size {
            Some(mon) => egui::pos2(
                (mon.x - size.0) / 2.0,
                (mon.y - size.1) / 2.0,
            ),
            None => egui::pos2(100.0, 100.0),
        }
    }

    /// Attach the frosted-glass NSVisualEffectView to the window (macOS
    /// only; a no-op elsewhere). Failures (window not created yet) are
    /// logged once — this runs every tick.
    fn apply_glass(&mut self, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        if let Err(e) = crate::platform::macos::apply_vibrancy() {
            if !self.glass_warned {
                self.glass_warned = true;
                eprintln!("vibrancy not applied (will retry): {e}");
            }
            return;
        }
        #[cfg(target_os = "macos")]
        {
            self.glass_warned = false;
        }
    }

    pub fn hide_launcher(&mut self, ctx: &egui::Context) {
        if !self.launcher_visible {
            return;
        }
        self.launcher_visible = false;
        // Closing the launcher stops any dictation playback — audio should
        // not keep playing from a hidden window.
        self.stop_all_playback();
        self.apply_window_mode(ctx);
    }

    // -----------------------------------------------------------------
    // List loading helpers
    // -----------------------------------------------------------------

    pub fn load_todos(&mut self) {
        self.todos = todos::list("all");
        self.active = 0;
    }

    pub fn load_notes(&mut self, query: &str) {
        self.notes = notes::list(Some(query));
        self.active = 0;
        self.sel_note = None;
    }

    pub fn load_dictations(&mut self) {
        self.dictations = dictations::list(None);
        self.active = 0;
        self.sel_dict = None;
        // Existing playback handles are kept: a paused dictation still
        // resumes from its position after the list is reloaded.
    }

    /// Stop every dictation playback (launcher hidden / list reloaded).
    pub fn stop_all_playback(&mut self) {
        for (_, p) in &self.playbacks {
            p.stop();
        }
        self.playbacks.clear();
    }

    /// Stop and forget one dictation's playback (row collapsed / deleted).
    pub fn stop_dictation(&mut self, id: &str) {
        if let Some(i) = self.playbacks.iter().position(|(pid, _)| pid == id) {
            let (_, p) = self.playbacks.remove(i);
            p.stop();
        }
    }

    /// Pause every playing dictation except `id` (pass `""` to pause all).
    /// Paused dictations keep their position and resume where they were
    /// when played again.
    pub fn pause_playbacks_except(&mut self, id: &str) {
        for (pid, p) in &self.playbacks {
            if pid != id {
                p.set_paused(true);
            }
        }
    }

    /// The playback handle for a dictation, if it is loaded.
    pub fn playback_handle(&self, id: &str) -> Option<&audio::PlaybackHandle> {
        self.playbacks
            .iter()
            .find(|(pid, _)| pid == id)
            .map(|(_, h)| h)
    }

    // -----------------------------------------------------------------
    // Chat (launcher AI view)
    // -----------------------------------------------------------------

    pub fn send_chat(&mut self, text: &str) {
        let text = text.trim().to_string();
        if text.is_empty() || self.chat_busy {
            return;
        }
        self.view = View::Chat;
        self.query.clear();

        let mut turns = std::mem::take(&mut self.chat);
        turns.push(ChatTurn {
            role: "user".into(),
            content: text.clone(),
            ..ChatTurn::default()
        });
        turns.push(ChatTurn {
            role: "assistant".into(),
            streaming: true,
            waiting: true,
            started_at: Some(Instant::now()),
            ..ChatTurn::default()
        });
        // history = all completed turns except the placeholder assistant.
        let history: Vec<ai::ChatMsg> = turns[..turns.len() - 1]
            .iter()
            .map(|t| ai::ChatMsg {
                role: t.role.clone(),
                content: t.content.clone(),
            })
            .collect();
        self.chat = turns;
        self.chat_busy = true;

        let settings = self.shared.settings.lock().unwrap().clone();
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let (ev_tx, ev_rx) = channel::<StreamEvent>();
            let forward = shared.events_tx.clone();
            std::thread::spawn(move || {
                for ev in ev_rx {
                    if forward.send(UiEvent::ChatEvent(ev)).is_err() {
                        break;
                    }
                    shared.repaint();
                }
            });
            let _ = ai::run_agent_stream(
                &settings.openai_api_key,
                &settings.chat_base_url,
                &settings.chat_model,
                &history,
                &text,
                &ev_tx,
            );
        });
    }

    fn apply_chat_event(&mut self, ev: StreamEvent) {
        if self.chat.is_empty() {
            return;
        }
        let last = self.chat.last_mut().unwrap();
        last.waiting = false;
        match ev {
            StreamEvent::Start => {}
            StreamEvent::Thinking { text } => last.thinking.push_str(&text),
            StreamEvent::Delta { text } => last.content.push_str(&text),
            StreamEvent::Tool { name } => last.tools.push(name),
            StreamEvent::Done { text } => {
                if !text.is_empty() {
                    last.content = text;
                }
                last.streaming = false;
            }
            StreamEvent::Error { message } => {
                last.content = message;
                last.error = true;
                last.streaming = false;
            }
        }
        if !last.streaming {
            self.chat_busy = false;
        }
    }

    pub fn clear_chat(&mut self) {
        self.chat.clear();
        self.dictate_thread = false;
        self.view = View::Search;
        self.need_focus = true;
    }

    // -----------------------------------------------------------------
    // Dictation state machine (native: cpal mic instead of MediaRecorder)
    // -----------------------------------------------------------------

    fn recording_active(&self) -> bool {
        let d = self.shared.dictate.lock().unwrap();
        d.open && (d.phase == DPhase::Listening || d.phase == DPhase::Recording)
    }

    pub fn open_dictate(&mut self, mode: DictateMode, ctx: &egui::Context) {
        match mode {
            DictateMode::Supacast if self.dictate_thread => {
                // Continuing the dictated thread: seed the agent history
                // with what's currently in the chat view (it may include
                // typed follow-ups as well as dictated turns).
                let seeded: Vec<ai::ChatMsg> = self
                    .chat
                    .iter()
                    .filter(|t| !t.streaming && !t.content.trim().is_empty())
                    .map(|t| ai::ChatMsg {
                        role: t.role.clone(),
                        content: t.content.clone(),
                    })
                    .collect();
                *self.shared.dictate_history.lock().unwrap() = seeded;
            }
            DictateMode::Supacast => {
                // A brand-new thread: forget any old history.
                self.shared.dictate_history.lock().unwrap().clear();
            }
            DictateMode::Text => {}
        }
        #[cfg(target_os = "macos")]
        paste_focus::capture(); // remember the user's paste target
        let mut d = self.shared.dictate.lock().unwrap();
        d.reset_for(mode);
        drop(d);
        self.shared.set_enter_capture(true);
        self.apply_window_mode(ctx);
    }

    pub fn close_dictate(&mut self, ctx: &egui::Context) {
        let mut d = self.shared.dictate.lock().unwrap();
        if !d.open {
            return;
        }
        d.open = false;
        // Cancel any in-flight recording.
        if d.phase == DPhase::Listening || d.phase == DPhase::Recording {
            *self.shared.recorder.lock().unwrap() = None; // drop -> stop mic
        }
        *d = DictateState::default();
        drop(d);
        self.shared.set_enter_capture(false);
        self.apply_window_mode(ctx);
    }

    fn dictate_key_down(&mut self) {
        // Same physical press is reported both by the global hotkey and the
        // focused window path — the `enter_held` guard dedupes them.
        if self.shared.enter_held.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut d = self.shared.dictate.lock().unwrap();
        if !d.open
            || (d.phase != DPhase::Idle && d.phase != DPhase::Done && d.phase != DPhase::Error)
        {
            return;
        }
        d.transcript.clear();
        d.answer.clear();
        d.error.clear();
        d.phase = DPhase::Listening;
        drop(d);

        // Long-press: record only after HOLD_MS with Enter still down.
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(HOLD_MS));
            if shared.enter_held.load(Ordering::SeqCst) {
                let _ = shared.events_tx.send(UiEvent::StartRecording);
            }
            shared.repaint();
        });
        self.shared.repaint();
    }

    fn dictate_key_up(&mut self) {
        self.shared.enter_held.store(false, Ordering::SeqCst);
        let d = self.shared.dictate.lock().unwrap();
        let phase = d.phase;
        drop(d);
        match phase {
            DPhase::Recording => {
                let _ = self.shared.events_tx.send(UiEvent::StopRecording(false));
            }
            DPhase::Listening => {
                // Released before the long-press threshold — ignore entirely.
                let mut d = self.shared.dictate.lock().unwrap();
                d.phase = DPhase::Idle;
            }
            _ => {}
        }
        self.shared.repaint();
    }

    fn start_recording(&mut self) {
        // Enter was released before the mic finished starting up.
        let released = !self.shared.enter_held.load(Ordering::SeqCst);

        match audio::Recorder::start() {
            Ok(recorder) => {
                *self.shared.recorder.lock().unwrap() = Some(recorder);
                let mut d = self.shared.dictate.lock().unwrap();
                d.phase = DPhase::Recording;
                d.record_start = Some(Instant::now());
                drop(d);

                // 60s hard cap.
                let shared = self.shared.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(MAX_RECORD_MS));
                    let d = shared.dictate.lock().unwrap();
                    if d.phase == DPhase::Recording {
                        drop(d);
                        let _ = shared.events_tx.send(UiEvent::StopRecording(false));
                    }
                });
                if released {
                    let _ = self.shared.events_tx.send(UiEvent::StopRecording(false));
                }
            }
            Err(e) => {
                let mut d = self.shared.dictate.lock().unwrap();
                d.phase = DPhase::Error;
                d.error = e;
            }
        }
        self.shared.repaint();
    }

    fn stop_recording(&mut self, ctx: &egui::Context, cancelled: bool) {
        let recorder = self.shared.recorder.lock().unwrap().take();
        let mut d = self.shared.dictate.lock().unwrap();
        if cancelled || recorder.is_none() {
            drop(d);
            self.close_dictate(ctx);
            return;
        }
        let (samples, rate) = recorder.unwrap().stop();
        let duration_ms = d
            .record_start
            .map(|t| t.elapsed().as_millis() as u64);
        let mode = d.mode;
        d.phase = DPhase::Transcribing;
        drop(d);
        self.shared.repaint();

        // Transcribe + post-process on a worker thread; results flow back
        // as events (PasteToFocused must run on the main thread).
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            process_dictation(shared, samples, rate, mode, duration_ms);
        });
    }

    /// Copy dictated text, restore the previously-used app, and paste at
    /// its cursor. Runs on the main thread: enigo's macOS implementation
    /// queries HIToolbox APIs that are dispatch-asserted to the main queue.
    fn paste_to_focused(&mut self, text: &str) {
        if let Err(e) = clipboard_hist::copy_to_clipboard(text) {
            eprintln!("clipboard copy failed: {e}");
        }
        #[cfg(target_os = "macos")]
        {
            // Bring the original paste target back to the front so the
            // keystroke lands in the user's text field. AppKit wants the
            // main thread — we are on it.
            paste_focus::restore();
        }
        std::thread::sleep(Duration::from_millis(300));
        if paste_keystroke().is_err() {
            crate::platform::notify(
                "Supacast dictate",
                "Copied to clipboard — paste with Cmd/Ctrl+V",
            );
        }
    }

    // -----------------------------------------------------------------
    // Settings save
    // -----------------------------------------------------------------

    /// Close the settings panel and return the window to its next state.
    pub fn close_settings(&mut self, ctx: &egui::Context) {
        self.settings_open = false;
        self.apply_window_mode(ctx);
    }

    pub fn save_settings(&mut self) {
        let shortcut = settings::normalize_shortcut(&self.set_shortcut);
        let mut current = self.shared.settings.lock().unwrap().clone();

        let changed_shortcut = shortcut != current.shortcut;
        current.shortcut = shortcut;
        current.openai_api_key = self.set_api_key.trim().to_string();
        current.chat_base_url = self
            .set_base_url
            .trim()
            .trim_end_matches('/')
            .to_string();
        current.chat_model = self.set_model.trim().to_string();

        match settings::save(&current) {
            Ok(()) => {
                *self.shared.settings.lock().unwrap() = current.clone();
                self.set_shortcut = current.shortcut.clone();
                self.settings_status = SettingsStatus::Saved;
                // Only re-register the global shortcut when it changed.
                if changed_shortcut {
                    self.shared
                        .set_toggle(&current.shortcut.clone());
                }
            }
            Err(e) => {
                self.settings_status = SettingsStatus::Error(e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Dictation worker
// ---------------------------------------------------------------------------

/// Encode, transcribe and dispatch a finished recording. Runs off-thread;
/// talks back via UiEvents + the shared dictate state.
fn process_dictation(
    shared: Arc<Shared>,
    samples: Vec<f32>,
    rate: u32,
    mode: DictateMode,
    duration_ms: Option<u64>,
) {
    let wav = audio::encode_wav(&samples, rate);
    let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &wav);

    // Guard against silent captures: with macOS microphone permission
    // denied, CoreAudio hands us pure zeros and the transcription comes
    // back empty ("Nothing heard") with no hint why. Detect it here and
    // tell the user what to fix.
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak < 0.001 {
        let mut d = shared.dictate.lock().unwrap();
        d.phase = DPhase::Error;
        d.error = "No microphone input captured. Check that Supacast has microphone access (System Settings → Privacy & Security → Microphone) and that the right input device is selected."
            .to_string();
        drop(d);
        shared.repaint();
        return;
    }

    let settings = shared.settings.lock().unwrap().clone();
    let result = ai::transcribe(
        &settings.openai_api_key,
        &settings.chat_base_url,
        &b64,
        "audio/wav",
    );

    match result {
        Ok(text) => {
            let text = text.trim().to_string();
            {
                let mut d = shared.dictate.lock().unwrap();
                if text.is_empty() {
                    d.phase = DPhase::Error;
                    d.error = "Nothing heard".to_string();
                    drop(d);
                    shared.repaint();
                    return;
                }
                d.transcript = text.clone();
            }

            // Save the recording + transcript so it shows up under "dictations".
            let persist = |answer: Option<String>| -> Result<(), String> {
                dictations::save_bytes(&wav, "audio/wav", &text, answer, duration_ms).map(|_| ())
            };

            match mode {
                DictateMode::Text => {
                    persist(None).unwrap_or_else(|e| eprintln!("save failed: {e}"));
                    let _ = shared.events_tx.send(UiEvent::PasteToFocused(text));
                    let mut d = shared.dictate.lock().unwrap();
                    d.phase = DPhase::Done;
                }
                DictateMode::Supacast => {
                    {
                        let mut d = shared.dictate.lock().unwrap();
                        d.phase = DPhase::Thinking;
                    }
                    shared.repaint();
                    // Continue the thread: everything dictated (or typed in
                    // the chat view) so far is passed as history.
                    let history = shared.dictate_history.lock().unwrap().clone();
                    match ai::run_agent_with_history(
                        &settings.openai_api_key,
                        &settings.chat_base_url,
                        &settings.chat_model,
                        &history,
                        &text,
                    ) {
                        Ok(reply) => {
                            // Remember the exchange so the next dictate in
                            // this thread keeps its context.
                            {
                                let mut h = shared.dictate_history.lock().unwrap();
                                h.push(ai::ChatMsg {
                                    role: "user".into(),
                                    content: text.clone(),
                                });
                                h.push(ai::ChatMsg {
                                    role: "assistant".into(),
                                    content: reply.clone(),
                                });
                            }
                            // Hand the Q&A to the launcher: it opens showing
                            // this exchange. Saved after the reply so the
                            // answer is stored with the recording.
                            persist(Some(reply.clone()))
                                .unwrap_or_else(|e| eprintln!("save failed: {e}"));
                            let _ = shared
                                .events_tx
                                .send(UiEvent::DictateAnswer { message: text, answer: reply });
                            let mut d = shared.dictate.lock().unwrap();
                            d.phase = DPhase::Done;
                        }
                        Err(e) => {
                            // Still keep the recording + transcript even if
                            // the agent failed.
                            persist(None).ok();
                            let mut d = shared.dictate.lock().unwrap();
                            d.phase = DPhase::Error;
                            d.error = e;
                        }
                    }
                }
            }
        }
        Err(e) => {
            let mut d = shared.dictate.lock().unwrap();
            d.phase = DPhase::Error;
            d.error = e;
        }
    }
    shared.repaint();
}

// ---------------------------------------------------------------------------
// Paste keystroke (port of the Tauri command)
// ---------------------------------------------------------------------------

/// Simulate a Cmd/Ctrl+V keystroke so the paste lands in whatever input
/// had focus in the previously-used app.
fn paste_keystroke() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta // Command on macOS
    } else {
        Key::Control
    };
    enigo.key(modifier, Direction::Press).map_err(|e| e.to_string())?;
    enigo.key(Key::Unicode('v'), Direction::Click).map_err(|e| e.to_string())?;
    enigo.key(modifier, Direction::Release).map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Small formatting helpers shared by the UI modules
// ---------------------------------------------------------------------------

pub fn fmt_due(ts: Option<chrono::DateTime<Local>>) -> String {
    match ts {
        Some(t) => t.format("%b %-d %-I:%M %p").to_string(),
        None => String::new(),
    }
}

#[allow(dead_code)] // small convenience helper
pub fn now() -> chrono::DateTime<Local> {
    Local::now()
}
