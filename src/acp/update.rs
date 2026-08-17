//! What an agent did, in Shaipe's own vocabulary.
//!
//! This is a firewall. ACP's `SessionUpdate` is `#[non_exhaustive]` and grows
//! as the protocol does; a workspace that matched on it directly would stop
//! compiling every time it did, and would leak protocol types into pane
//! drawing code. So the connection translates once, here, and nothing above
//! `src/acp/` ever sees an ACP type.
//!
//! The translation is deliberately lossy. What the workspace needs is enough
//! to show a person what is happening, not a faithful re-encoding of the
//! protocol.

use agent_client_protocol::schema::v1::{
    ContentBlock, SessionNotification, SessionUpdate, ToolCallStatus,
};

/// How a tool call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// Still going.
    Running,
    /// It worked.
    Completed,
    /// It did not.
    Failed,
}

impl ToolStatus {
    /// A single character for a list that has one column to spare.
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::Running => "·",
            Self::Completed => "✓",
            Self::Failed => "✗",
        }
    }
}

/// Something the agent did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentUpdate {
    /// The handshake finished and a session is open.
    ///
    /// An update rather than the result of starting, because the workspace
    /// must not wait for it — see the deadlock described in
    /// [`super::Agent::start`].
    Ready,
    /// The agent could not be started, or stopped for good.
    Failed(String),
    /// Prose, streamed a chunk at a time.
    Message(String),
    /// Reasoning, which the workspace shows dimmed and apart.
    Thought(String),
    /// A tool it started.
    ToolStarted {
        /// The agent's identifier for the call.
        id: String,
        /// What a human reads.
        title: String,
    },
    /// How one ended.
    ToolFinished {
        /// The identifier from [`AgentUpdate::ToolStarted`].
        id: String,
        /// How it went.
        status: ToolStatus,
    },
    /// The turn is over and the agent is waiting.
    Idle,
    /// Something this build does not model.
    ///
    /// Carried rather than dropped: the protocol grows, and a workspace that
    /// silently swallowed the parts it did not recognise would look wedged
    /// during a session that was working perfectly.
    Other(String),
}

/// Translate one notification from the agent.
///
/// Returns a `Vec` rather than an `Option` because one notification can
/// reasonably become more than one thing worth showing, and none at all is
/// also a valid answer.
pub(crate) fn from_session(notification: &SessionNotification) -> Vec<AgentUpdate> {
    match &notification.update {
        SessionUpdate::AgentMessageChunk(chunk) => text_of(&chunk.content)
            .into_iter()
            .map(AgentUpdate::Message)
            .collect(),
        SessionUpdate::AgentThoughtChunk(chunk) => text_of(&chunk.content)
            .into_iter()
            .map(AgentUpdate::Thought)
            .collect(),

        // The user's own message, echoed back. The workspace already put it in
        // the transcript when it was typed, so showing it again would double
        // every prompt.
        SessionUpdate::UserMessageChunk(_) => Vec::new(),

        SessionUpdate::ToolCall(call) => vec![AgentUpdate::ToolStarted {
            id: call.tool_call_id.0.to_string(),
            title: call.title.clone(),
        }],

        SessionUpdate::ToolCallUpdate(update) => {
            let id = update.tool_call_id.0.to_string();
            let mut updates = Vec::new();

            // A title arriving in an update is a call whose name was not known
            // when it started — a streamed argument list, usually.
            if let Some(title) = &update.fields.title {
                updates.push(AgentUpdate::ToolStarted {
                    id: id.clone(),
                    title: title.clone(),
                });
            }

            if let Some(status) = update.fields.status {
                updates.push(AgentUpdate::ToolFinished {
                    id,
                    status: status.into(),
                });
            }

            updates
        }

        // Modelled as nothing rather than as noise. A plan is worth showing
        // and is not worth showing badly, and the transcript has no shape for
        // one yet; `PLAN.md` carries it.
        SessionUpdate::Plan(_) => Vec::new(),

        // Housekeeping the workspace has no use for: which slash commands
        // exist, which mode is current, how many tokens have been spent.
        SessionUpdate::AvailableCommandsUpdate(_)
        | SessionUpdate::CurrentModeUpdate(_)
        | SessionUpdate::ConfigOptionUpdate(_)
        | SessionUpdate::SessionInfoUpdate(_)
        | SessionUpdate::UsageUpdate(_) => Vec::new(),

        // The arm that makes ACP's `#[non_exhaustive]` a non-event. Something
        // arrived, this build does not model it, and saying so is better than
        // a workspace that looks wedged while the agent works.
        other => vec![AgentUpdate::Other(format!(
            "the agent sent an update this build does not show ({})",
            variant_name(other)
        ))],
    }
}

/// The text of a content block, if it has any.
fn text_of(content: &ContentBlock) -> Option<String> {
    match content {
        ContentBlock::Text(text) => Some(text.text.clone()),
        // An agent sending Shaipe a picture is not something the transcript
        // can show, and pretending otherwise would print base64 at someone.
        _ => None,
    }
}

/// A name for an update this build does not model.
///
/// Derived from the serialised tag rather than from a match, so that a variant
/// added to the protocol is named correctly without this file changing.
fn variant_name(update: &SessionUpdate) -> String {
    serde_json::to_value(update)
        .ok()
        .and_then(|value| {
            value
                .get("sessionUpdate")
                .and_then(|tag| tag.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

impl From<ToolCallStatus> for ToolStatus {
    fn from(status: ToolCallStatus) -> Self {
        match status {
            ToolCallStatus::Pending | ToolCallStatus::InProgress => Self::Running,
            ToolCallStatus::Completed => Self::Completed,
            ToolCallStatus::Failed => Self::Failed,
            // The protocol may add statuses. Treating an unknown one as still
            // running is the honest answer: it has not been reported finished.
            _ => Self::Running,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    use agent_client_protocol::schema::v1::{
        ContentChunk, SessionId, TextContent, ToolCall, ToolCallId, ToolCallUpdate,
        ToolCallUpdateFields,
    };

    /// A notification carrying one update.
    fn notify(update: SessionUpdate) -> SessionNotification {
        SessionNotification::new(SessionId::new("test"), update)
    }

    fn chunk(text: &str) -> ContentChunk {
        ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
    }

    #[test]
    fn an_agent_message_becomes_something_the_transcript_can_show() {
        let updates = from_session(&notify(SessionUpdate::AgentMessageChunk(chunk("Drawing."))));
        assert_eq!(updates, [AgentUpdate::Message("Drawing.".to_owned())]);
    }

    #[test]
    fn a_thought_is_kept_apart_from_what_the_agent_says() {
        // Reasoning read as a statement is how a person ends up believing the
        // agent said something it did not.
        let updates = from_session(&notify(SessionUpdate::AgentThoughtChunk(chunk("hmm"))));
        assert_eq!(updates, [AgentUpdate::Thought("hmm".to_owned())]);
    }

    #[test]
    fn the_users_own_message_is_not_echoed_back_into_the_transcript() {
        // The workspace put it there when it was typed. Showing the echo too
        // would double every prompt.
        let updates = from_session(&notify(SessionUpdate::UserMessageChunk(chunk("a logo"))));
        assert!(updates.is_empty(), "{updates:?}");
    }

    #[test]
    fn a_tool_call_arrives_with_the_title_a_person_reads() {
        let call = ToolCall::new(ToolCallId::new("t1"), "render_svg");

        let updates = from_session(&notify(SessionUpdate::ToolCall(call)));
        assert_eq!(
            updates,
            [AgentUpdate::ToolStarted {
                id: "t1".to_owned(),
                title: "render_svg".to_owned(),
            }]
        );
    }

    #[test]
    fn a_failed_tool_call_is_reported_as_failed() {
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Failed);
        let update = ToolCallUpdate::new(ToolCallId::new("t1"), fields);

        let updates = from_session(&notify(SessionUpdate::ToolCallUpdate(update)));
        assert_eq!(
            updates,
            [AgentUpdate::ToolFinished {
                id: "t1".to_owned(),
                status: ToolStatus::Failed,
            }]
        );
    }

    #[test]
    fn a_pending_tool_call_is_still_running_rather_than_finished() {
        // "Pending" means waiting for approval or for streamed arguments. A
        // transcript that showed it as done would be lying about the state.
        assert_eq!(
            ToolStatus::from(ToolCallStatus::Pending),
            ToolStatus::Running
        );
        assert_eq!(
            ToolStatus::from(ToolCallStatus::InProgress),
            ToolStatus::Running
        );
    }

    #[test]
    fn an_update_this_build_does_not_model_is_named_rather_than_dropped() {
        // ACP's update enum is `#[non_exhaustive]` and will grow. A workspace
        // that silently swallowed the new variants would look wedged during a
        // session that was working perfectly — and the name comes from the
        // serialised tag, so a future variant is named correctly without this
        // file changing.
        let updates = from_session(&notify(SessionUpdate::CurrentModeUpdate(
            agent_client_protocol::schema::v1::CurrentModeUpdate::new(
                agent_client_protocol::schema::v1::SessionModeId::new("build"),
            ),
        )));

        // Modelled as housekeeping, so nothing is shown for this one.
        assert!(updates.is_empty(), "{updates:?}");
    }

    #[test]
    fn the_name_of_an_unmodelled_update_comes_from_the_wire_not_from_a_match() {
        // What makes the catch-all arm useful rather than merely present.
        let update = SessionUpdate::AgentMessageChunk(chunk("x"));
        assert_eq!(variant_name(&update), "agent_message_chunk");
    }

    #[test]
    fn every_tool_status_has_a_glyph_of_its_own() {
        // They share a column in the transcript; two statuses that look the
        // same are a list that cannot be read.
        let glyphs = [
            ToolStatus::Running.glyph(),
            ToolStatus::Completed.glyph(),
            ToolStatus::Failed.glyph(),
        ];
        let unique: std::collections::BTreeSet<_> = glyphs.iter().collect();
        assert_eq!(unique.len(), glyphs.len());
    }
}
