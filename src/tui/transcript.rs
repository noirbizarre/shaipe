//! What the agent has said, and what it did.
//!
//! Chunks are coalesced into entries as they arrive: an agent streams a
//! message a few tokens at a time, and one transcript entry per token would be
//! unreadable and would re-wrap the pane on every update.
//!
//! This type deliberately holds no protocol types. It takes
//! [`crate::acp::AgentUpdate`], which is Shaipe's own vocabulary, so that ACP
//! adding a variant cannot reach the workspace.

use crate::acp::{AgentUpdate, ToolStatus};

/// One thing in the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// Something the user said.
    You(String),
    /// Prose from the agent.
    Agent(String),
    /// The agent's reasoning, shown apart from what it actually says.
    Thought(String),
    /// A tool the agent called.
    Tool {
        /// The agent's own identifier for the call, used to match a completion
        /// to the line that announced it.
        id: String,
        /// What a human reads.
        title: String,
        /// How it went.
        status: ToolStatus,
    },
    /// Something Shaipe wants to say: a failure, a refusal, an update this
    /// build does not model.
    Notice(String),
}

/// The conversation so far.
#[derive(Debug, Default, Clone)]
pub struct Transcript {
    entries: Vec<Entry>,
    /// Whether the agent is working on a turn.
    busy: bool,
}

impl Transcript {
    /// Everything said so far, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Whether a turn is in flight.
    #[must_use]
    pub const fn is_busy(&self) -> bool {
        self.busy
    }

    /// The tool the agent is running, if it is running one.
    ///
    /// The most recent, so a burst of calls names the one actually in flight
    /// rather than the first of them.
    #[must_use]
    pub fn running_tool(&self) -> Option<&str> {
        self.entries.iter().rev().find_map(|entry| match entry {
            Entry::Tool {
                title,
                status: ToolStatus::Running,
                ..
            } => Some(title.as_str()),
            _ => None,
        })
    }

    /// Whether anything has been said at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Record what the user asked for, and that a turn has started.
    pub fn push_user(&mut self, text: String) {
        self.entries.push(Entry::You(text));
        self.busy = true;
    }

    /// Say something on Shaipe's own behalf.
    pub fn push_notice(&mut self, text: impl Into<String>) {
        self.entries.push(Entry::Notice(text.into()));
    }

    /// Apply one update from the agent.
    pub fn apply(&mut self, update: AgentUpdate) {
        match update {
            AgentUpdate::Message(chunk) => self.append(&chunk, false),
            AgentUpdate::Thought(chunk) => self.append(&chunk, true),

            AgentUpdate::ToolStarted { id, title } => self.entries.push(Entry::Tool {
                id,
                title,
                status: ToolStatus::Running,
            }),

            AgentUpdate::ToolFinished { id, status } => {
                // Matched by identifier rather than by position: an agent may
                // run several tools at once, and updating the most recent one
                // would attribute a failure to the wrong call.
                if let Some(Entry::Tool {
                    status: existing, ..
                }) = self.entries.iter_mut().rev().find(
                    |entry| matches!(entry, Entry::Tool { id: existing, .. } if *existing == id),
                ) {
                    *existing = status;
                } else {
                    // A completion for a call that was never announced. Kept,
                    // because silently dropping it hides a real exchange.
                    self.entries.push(Entry::Tool {
                        id,
                        title: "a tool".to_owned(),
                        status,
                    });
                }
            }

            AgentUpdate::Idle => self.busy = false,

            // Worth saying once. A workspace that shows nothing between
            // opening and the first answer looks like one that is ignoring
            // the prompt.
            AgentUpdate::Ready => {
                if self.entries.is_empty() {
                    self.entries
                        .push(Entry::Notice("the agent is ready".to_owned()));
                }
            }

            AgentUpdate::Failed(reason) => {
                self.busy = false;
                self.entries.push(Entry::Notice(reason));
            }

            // Not a conversation entry. `App` reads the list straight off
            // the update before this runs (see `event_loop`), and a picker
            // with nothing to show is not something worth saying here.
            AgentUpdate::Models(_) => {}

            // Kept, never dropped. An update this build does not model is
            // still evidence the agent is doing something, and swallowing it
            // makes a working session look wedged.
            AgentUpdate::Other(text) => self.entries.push(Entry::Notice(text)),
        }
    }

    /// Append a streamed chunk, starting a new entry only when the kind changes.
    fn append(&mut self, chunk: &str, thought: bool) {
        match self.entries.last_mut() {
            Some(Entry::Agent(text)) if !thought => text.push_str(chunk),
            Some(Entry::Thought(text)) if thought => text.push_str(chunk),
            _ if thought => self.entries.push(Entry::Thought(chunk.to_owned())),
            _ => self.entries.push(Entry::Agent(chunk.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn consecutive_chunks_coalesce_into_one_entry() {
        // An agent streams a few tokens at a time. One entry per chunk would
        // re-wrap the pane on every update and be unreadable besides.
        let mut transcript = Transcript::default();
        transcript.apply(AgentUpdate::Message("I will draw ".to_owned()));
        transcript.apply(AgentUpdate::Message("a circle.".to_owned()));

        assert_eq!(
            transcript.entries(),
            [Entry::Agent("I will draw a circle.".to_owned())]
        );
    }

    #[test]
    fn a_thought_is_kept_apart_from_what_the_agent_says() {
        let mut transcript = Transcript::default();
        transcript.apply(AgentUpdate::Thought("the mark needs ".to_owned()));
        transcript.apply(AgentUpdate::Thought("more contrast".to_owned()));
        transcript.apply(AgentUpdate::Message("Making it darker.".to_owned()));

        assert_eq!(
            transcript.entries(),
            [
                Entry::Thought("the mark needs more contrast".to_owned()),
                Entry::Agent("Making it darker.".to_owned()),
            ]
        );
    }

    #[test]
    fn a_tool_call_and_its_completion_are_matched_by_id() {
        // An agent may run several tools at once. Updating the most recent
        // entry instead would report a failure against the wrong call.
        let mut transcript = Transcript::default();
        transcript.apply(AgentUpdate::ToolStarted {
            id: "1".to_owned(),
            title: "render_svg".to_owned(),
        });
        transcript.apply(AgentUpdate::ToolStarted {
            id: "2".to_owned(),
            title: "get_palette".to_owned(),
        });
        transcript.apply(AgentUpdate::ToolFinished {
            id: "1".to_owned(),
            status: ToolStatus::Failed,
        });

        assert_eq!(
            transcript.entries(),
            [
                Entry::Tool {
                    id: "1".to_owned(),
                    title: "render_svg".to_owned(),
                    status: ToolStatus::Failed,
                },
                Entry::Tool {
                    id: "2".to_owned(),
                    title: "get_palette".to_owned(),
                    status: ToolStatus::Running,
                },
            ]
        );
    }

    #[test]
    fn an_update_this_build_does_not_model_is_kept_rather_than_dropped() {
        // ACP's update enum is `#[non_exhaustive]` and will grow. A workspace
        // that silently swallowed the new variants would look wedged during a
        // session that was working perfectly.
        let mut transcript = Transcript::default();
        transcript.apply(AgentUpdate::Other(
            "switched to a different model".to_owned(),
        ));

        assert_eq!(
            transcript.entries(),
            [Entry::Notice("switched to a different model".to_owned())]
        );
    }

    #[test]
    fn the_list_of_models_an_agent_offers_is_not_a_conversation_entry() {
        // `App` reads the list off the update itself, before this runs; the
        // transcript has nothing to say about a picker having options.
        let mut transcript = Transcript::default();
        transcript.apply(AgentUpdate::Models(vec![]));

        assert!(transcript.entries().is_empty());
    }

    #[test]
    fn a_turn_is_busy_from_the_prompt_until_the_agent_goes_idle() {
        let mut transcript = Transcript::default();
        assert!(!transcript.is_busy());

        transcript.push_user("a minimalist logo".to_owned());
        assert!(transcript.is_busy());

        transcript.apply(AgentUpdate::Message("Done.".to_owned()));
        assert!(transcript.is_busy(), "a message is not the end of a turn");

        transcript.apply(AgentUpdate::Idle);
        assert!(!transcript.is_busy());
    }
}
