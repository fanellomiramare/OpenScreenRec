//! Recording control shared between the UI thread and the tray/hotkey
//! command thread.
//!
//! The UI renders snapshots of the state and drains finished sessions;
//! the command thread (tray menu, global hotkeys) starts and stops
//! recordings. `request_stop` is deliberately non-blocking so no UI
//! frame ever waits on the mux step.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::recorder::{self, RecordRequest, Session};

/// What the toolbar needs to draw, copied out under a short lock.
#[derive(Clone, Debug)]
pub struct RecordingView {
    pub recording: bool,
    pub paused: bool,
    pub countdown: Option<Duration>,
    pub elapsed: Duration,
    pub last_result: Option<Result<PathBuf, String>>,
}

#[derive(Default)]
pub struct ControllerState {
    request: Option<RecordRequest>,
    session: Option<Session>,
    last_result: Option<Result<PathBuf, String>>,
}

impl ControllerState {
    pub fn poll(&mut self) {
        if self.session.is_some()
            && let Some(session) = self.session.as_mut()
            && let Some(result) = session.try_result()
        {
            self.session = None;
            self.last_result = Some(result);
        }
    }
}

/// Cheap clonable handle around the shared state.
#[derive(Clone, Default)]
pub struct Controller {
    state: Arc<Mutex<ControllerState>>,
    quit: Arc<AtomicBool>,
}

impl Controller {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lock(&self) -> MutexGuard<'_, ControllerState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Flag a real exit (tray Quit): the UI lets the window close
    /// instead of hiding it.
    pub fn request_quit(&self) {
        self.quit.store(true, Ordering::Relaxed);
    }

    pub fn is_quitting(&self) -> bool {
        self.quit.load(Ordering::Relaxed)
    }
}

impl ControllerState {
    /// Configure what the next recording will use.
    pub fn set_request(&mut self, request: RecordRequest) {
        self.request = Some(request);
    }

    pub fn is_recording(&self) -> bool {
        self.session.is_some()
    }

    /// Start a recording with the configured request.
    pub fn start(&mut self) -> Result<(), String> {
        if self.session.is_some() {
            return Ok(()); // already recording; treat as success
        }
        let request = self
            .request
            .as_ref()
            .ok_or("no recording request configured")?
            .clone();
        self.session = Some(recorder::start(request)?);
        Ok(())
    }

    /// Ask the current recording to finish; the result shows up in
    /// [`Self::poll`] once the encoder and mux are done.
    pub fn request_stop(&mut self) {
        if let Some(session) = &self.session {
            session.request_stop();
        }
    }

    /// Toggle recording: start when idle, request stop when recording.
    pub fn toggle_record(&mut self) -> Result<(), String> {
        if self.is_recording() {
            self.request_stop();
            Ok(())
        } else {
            self.start()
        }
    }

    pub fn toggle_pause(&mut self) {
        if let Some(session) = &self.session {
            if session.is_paused() {
                session.resume();
            } else {
                session.pause();
            }
        }
    }

    /// Blocking stop; only from app exit or the Quit command.
    pub fn stop_blocking(&mut self) {
        self.poll();
        if let Some(session) = self.session.take() {
            self.last_result = Some(session.join());
        }
    }

    /// Snapshot for rendering the toolbar.
    pub fn view(&mut self) -> RecordingView {
        self.poll();
        match self.session.as_ref() {
            Some(session) => RecordingView {
                recording: true,
                paused: session.is_paused(),
                countdown: session.countdown_remaining(),
                elapsed: session.recorded_elapsed(),
                last_result: self.last_result.clone(),
            },
            None => RecordingView {
                recording: false,
                paused: false,
                countdown: None,
                elapsed: Duration::ZERO,
                last_result: self.last_result.clone(),
            },
        }
    }
}
