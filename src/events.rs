//! Cross-thread event and shared dictate state types.
//!
//! The egui app runs everything UI on the main thread. Background threads
//! (hotkeys, tray, AI streaming, dictation post-processing, reminders)
//! communicate by sending `UiEvent`s through one mpsc channel that the
//! main loop drains every frame.

use crate::ai;
use std::sync::mpsc::Sender;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictateMode {
    /// Record & transcribe, then paste into the previous app.
    Text,
    /// Record & ask the Supacast agent; the answer shows in the launcher.
    Supacast,
}

/// Phase of the dictate ring, mirroring the old webview UI state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DPhase {
    /// Ring visible, waiting for the user to hold Enter.
    Idle,
    /// Enter held; mic is starting (short grace period).
    Listening,
    /// Recording.
    Recording,
    /// Transcribing the recording.
    Transcribing,
    /// Waiting for the agent answer (Supacast mode only).
    Thinking,
    /// Finished; shows the transcript.
    Done,
    /// Failed.
    Error,
}

#[derive(Debug)]
pub struct DictateState {
    pub open: bool,
    pub mode: DictateMode,
    pub phase: DPhase,
    pub transcript: String,
    pub answer: String,
    pub error: String,
    /// Wall-clock time the recording actually started, for the duration.
    pub record_start: Option<std::time::Instant>,
}

impl Default for DictateState {
    fn default() -> Self {
        Self {
            open: false,
            mode: DictateMode::Text,
            phase: DPhase::Idle,
            transcript: String::new(),
            answer: String::new(),
            error: String::new(),
            record_start: None,
        }
    }
}

impl DictateState {
    pub fn reset_for(&mut self, mode: DictateMode) {
        self.open = true;
        self.mode = mode;
        self.phase = DPhase::Idle;
        self.transcript.clear();
        self.answer.clear();
        self.error.clear();
        self.record_start = None;
    }
}

/// Events consumed by the main UI loop.
#[allow(dead_code)] // complete event API for future senders
pub enum UiEvent {
    /// Global shortcut pressed: show the launcher if hidden, hide if shown.
    ToggleLauncher,
    ShowLauncher,
    HideLauncher,
    OpenSettings,
    OpenDictate(DictateMode),
    /// Close the dictate ring (Esc / close button / finished flow).
    CloseDictate,
    /// Global Enter captured while the ring is open: (true = pressed).
    DictateKey(bool),
    /// The long-press threshold elapsed while Enter was still held.
    StartRecording,
    /// Enter released (or 60s cap hit): stop and process.
    /// `true` = cancelled (discard the recording and close).
    StopRecording(bool),
    /// Dictate (Text) finished: copy + restore focus + paste. Must run on
    /// the main thread (enigo on macOS is dispatch-asserted to the main
    /// queue — the original Tauri app did the same via run_on_main_thread).
    PasteToFocused(String),
    /// Dictate (Supacast) finished: show the Q&A in the launcher chat.
    DictateAnswer { message: String, answer: String },
    /// Streaming AI events for the launcher chat view.
    ChatEvent(ai::StreamEvent),
    /// App/file search finished. `gen` cancels stale results.
    SearchResults { gen: u64, results: Vec<crate::search::SearchResult> },
    Notify { title: String, body: String },
    CheckUpdates,
    Quit,
}

/// Sender alias used by worker threads.
pub type EventTx = Sender<UiEvent>;
