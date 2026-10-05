use anyhow::Result;

/// One text block of a user message; `cache` asks for a cache breakpoint after it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub text: String,
    pub cache: bool,
}

impl Block {
    pub fn new(text: impl Into<String>, cache: bool) -> Block {
        Block {
            text: text.into(),
            cache,
        }
    }
}

/// A model, reached through some harness or API.
pub trait Backend: Send + Sync {
    /// The harness or API the model runs in, recorded as the agent of what it writes.
    fn agent(&self) -> &str;

    /// The model's name, recorded with what it writes.
    fn model(&self) -> &str;

    /// A new conversation under the system prompt `system`.
    fn open(&self, system: &str) -> Result<Box<dyn Conversation>>;

    /// Ends every conversation still open.
    fn stop(&self) {}
}

/// One conversation: each user message gets the model's reply, in the same context.
pub trait Conversation: Send {
    fn say(&mut self, message: &[Block]) -> Result<String>;

    /// The id the harness gave this conversation, once known; empty if none.
    fn session(&self) -> String;
}
