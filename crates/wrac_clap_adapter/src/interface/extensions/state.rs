use crate::interface::{PluginResult, State};

/// The result of writing one prepared state payload to the host stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSaveOutcome {
    /// The complete payload was accepted by the host stream.
    Written,
    /// The host stream rejected the payload or accepted only a prefix.
    StreamWriteFailed,
}

/// A per-save completion object called exactly once after the ABI stream write attempt.
///
/// The object belongs to one `save_state` call; implementations must not use a shared "last saved"
/// slot when concurrent saves need distinct completion state. Completion runs on the ABI caller's
/// thread, so expensive work or thread-affine notifications should be scheduled elsewhere.
pub trait StateSaveCompletion: Send {
    /// `[non-realtime]`
    fn complete(self: Box<Self>, outcome: StateSaveOutcome);
}

/// A state payload and its optional per-save completion object.
///
/// Preparing this value does not mean that the host persisted the payload. Code that needs that
/// boundary must attach a completion and wait for `StateSaveOutcome::Written`.
pub struct PreparedStateSave {
    pub state: State,
    pub completion: Option<Box<dyn StateSaveCompletion>>,
}

impl PreparedStateSave {
    pub fn new(state: State) -> Self {
        Self {
            state,
            completion: None,
        }
    }

    pub fn with_completion(state: State, completion: Box<dyn StateSaveCompletion>) -> Self {
        Self {
            state,
            completion: Some(completion),
        }
    }
}

/// CLAP state extension.
pub trait PluginStateExtension: Send + Sync + 'static {
    /// Called from CLAP `state.save`. `[non-realtime & thread-safe]`
    fn save_state(&self) -> PluginResult<PreparedStateSave>;

    /// Called from CLAP `state.load`. `[non-realtime]`
    fn restore_state(&self, state: State) -> PluginResult<()>;
}
