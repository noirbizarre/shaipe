//! Reaching a project that something else owns.
//!
//! An MCP server and a workspace both need to call tools against *the same*
//! project, from different tasks, while the workspace is drawing frames with
//! `&mut self`. The obvious answer is a lock around the project. This module
//! is the reason there is not one — see ADR 009.
//!
//! ```text
//! MCP connection ──┐
//! MCP connection ──┼─> SessionHandle ──(channel)──> the task that owns
//! MCP connection ──┘                                the Project
//! ```
//!
//! One owner, reached only by message. Commands are serviced between frames,
//! which is the only moment at which mutating the project is safe, and they
//! are serviced in order, which is what [`crate::tools::Tool::call`] taking
//! `&mut Project` already requires.

use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::error::{Error, Result};
use crate::project::Project;
use crate::tools::{Registry, ToolDescriptor, ToolOutput};

/// How many commands may be in flight before an agent is made to wait.
///
/// Bounded, and small. An agent that outruns a human's workspace should
/// experience backpressure; an unbounded queue would instead let it build a
/// backlog of edits against a project it has stopped being able to see.
const DEPTH: usize = 16;

/// A request to the task that owns the project.
#[derive(Debug)]
pub enum SessionCommand {
    /// Describe every tool, for an MCP `tools/list`.
    List {
        /// Where to send them.
        reply: oneshot::Sender<Vec<ToolDescriptor>>,
    },
    /// Call one.
    Call {
        /// Which tool.
        name: String,
        /// Its arguments.
        input: Value,
        /// Where to send the result.
        reply: oneshot::Sender<Result<ToolOutput>>,
    },
}

/// What servicing a command did to the project.
///
/// Returned to the owner rather than acted on here, because this module knows
/// nothing about previews or dirty markers and must not start to. A workspace
/// reads `mutated` and decides for itself that its preview is now stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Applied {
    /// Whether the project changed.
    pub mutated: bool,
}

/// A way to reach a project without owning it.
///
/// Cheap to clone; one per MCP connection.
#[derive(Debug, Clone)]
pub struct SessionHandle {
    commands: mpsc::Sender<SessionCommand>,
}

impl SessionHandle {
    /// A handle, and the receiver whoever owns the project must service.
    ///
    /// This is the workspace's constructor: it already owns a `Project` and a
    /// loop to service the receiver from.
    #[must_use]
    pub fn channel() -> (Self, mpsc::Receiver<SessionCommand>) {
        let (commands, receiver) = mpsc::channel(DEPTH);
        (Self { commands }, receiver)
    }

    /// A handle to a project nothing else owns.
    ///
    /// This is standalone `shaipe mcp`: there is no workspace to service the
    /// channel, so a task is spawned that does nothing else. `save` writes the
    /// project back after any call that changed it — off unless asked for, for
    /// the same reason `write_svg` does not save.
    #[must_use]
    pub fn detached(project: Project, registry: Registry, save: bool) -> Self {
        let (handle, mut commands) = Self::channel();

        tokio::spawn(async move {
            let mut project = project;
            while let Some(command) = commands.recv().await {
                if serve(command, &registry, &mut project).mutated
                    && save
                    && let Err(error) = project.save()
                {
                    // Logged rather than returned: the tool call itself
                    // succeeded, and failing it now would tell the agent to
                    // retry an edit that has already been applied.
                    log::error!("could not save {}: {error}", project.path().display());
                }
            }
        });

        handle
    }

    /// Describe every tool.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SessionClosed`] if the project's owner has gone.
    pub async fn list(&self) -> Result<Vec<ToolDescriptor>> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(SessionCommand::List { reply })
            .await
            .map_err(|_| Error::SessionClosed)?;
        answer.await.map_err(|_| Error::SessionClosed)
    }

    /// Call one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::SessionClosed`] if the project's owner has gone, and
    /// otherwise whatever the tool returns.
    pub async fn call(&self, name: &str, input: Value) -> Result<ToolOutput> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(SessionCommand::Call {
                name: name.to_owned(),
                input,
                reply,
            })
            .await
            .map_err(|_| Error::SessionClosed)?;
        answer.await.map_err(|_| Error::SessionClosed)?
    }
}

/// Service one command against a project.
///
/// Shared by the workspace and by [`SessionHandle::detached`] so that the two
/// cannot drift into different meanings of the same tool call.
///
/// A dropped reply channel is ignored: it only means the caller gave up, which
/// is not this side's problem and certainly not worth failing over.
pub fn serve(command: SessionCommand, registry: &Registry, project: &mut Project) -> Applied {
    match command {
        SessionCommand::List { reply } => {
            drop(reply.send(registry.descriptors()));
            Applied { mutated: false }
        }
        SessionCommand::Call { name, input, reply } => {
            // Read before the call, because the tool is about to be consumed
            // by it, and asked of the tool rather than inferred from the
            // result: a failed `write_svg` changed nothing, and this is the
            // conservative answer for one that succeeded.
            let mutated = registry.get(&name).is_some_and(|tool| tool.mutates());
            let result = registry.call(&name, project, &input);
            let mutated = mutated && result.is_ok();

            drop(reply.send(result));
            Applied { mutated }
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;
    use crate::fixtures;

    fn session() -> SessionHandle {
        SessionHandle::detached(fixtures::project(), Registry::new(), false)
    }

    #[tokio::test]
    async fn a_detached_session_serves_a_tool_call() {
        let output = session().call("get_variants", Value::Null).await.unwrap();
        assert_eq!(output.value[0]["name"], "icon");
    }

    #[tokio::test]
    async fn a_detached_session_lists_the_same_tools_the_registry_holds() {
        let names: Vec<_> = session()
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|descriptor| descriptor.name)
            .collect();

        assert_eq!(names, Registry::new().names());
    }

    #[tokio::test]
    async fn a_tool_error_travels_back_rather_than_killing_the_session() {
        // The session has to survive a tool failing, or one bad argument from
        // a model would take the workspace's whole agent integration with it.
        let session = session();

        let error = session.call("vectorise", Value::Null).await.unwrap_err();
        assert!(matches!(error, Error::UnknownTool { .. }));

        // Still answering.
        assert!(session.call("get_variants", Value::Null).await.is_ok());
    }

    #[tokio::test]
    async fn a_call_to_a_closed_session_says_so_rather_than_hanging() {
        // What an MCP connection sees when the workspace it belongs to quits.
        // A hang here would leave an agent waiting forever on a tool call.
        let (handle, commands) = SessionHandle::channel();
        drop(commands);

        let error = handle.call("get_variants", Value::Null).await.unwrap_err();
        assert!(matches!(error, Error::SessionClosed), "{error:?}");
    }

    #[tokio::test]
    async fn an_owner_that_stops_answering_mid_call_is_not_a_hang_either() {
        // The other half: the command was accepted, and then the owner went
        // away without replying.
        let (handle, mut commands) = SessionHandle::channel();
        tokio::spawn(async move {
            let command = commands.recv().await;
            drop(command);
        });

        let error = handle.call("get_variants", Value::Null).await.unwrap_err();
        assert!(matches!(error, Error::SessionClosed), "{error:?}");
    }

    #[test]
    fn a_mutating_call_reports_that_the_project_changed() {
        // How a workspace learns its preview is stale without this module
        // knowing that previews exist.
        let mut project = fixtures::project();
        let (reply, _answer) = oneshot::channel();

        let applied = serve(
            SessionCommand::Call {
                name: "write_svg".to_owned(),
                input: json!({ "source": fixtures::PROJECT.replace("#f05032", "#00ff00") }),
                reply,
            },
            &Registry::new(),
            &mut project,
        );

        assert_eq!(applied, Applied { mutated: true });
    }

    #[test]
    fn a_mutating_call_that_failed_reports_no_change() {
        // A rejected `write_svg` changed nothing, and a workspace that
        // re-rendered anyway would flicker for every malformed document a
        // model produced.
        let mut project = fixtures::project();
        let (reply, _answer) = oneshot::channel();

        let applied = serve(
            SessionCommand::Call {
                name: "write_svg".to_owned(),
                input: json!({ "source": "<svg" }),
                reply,
            },
            &Registry::new(),
            &mut project,
        );

        assert_eq!(applied, Applied { mutated: false });
        assert_eq!(project.source(), fixtures::PROJECT);
    }

    #[test]
    fn a_read_only_call_reports_no_change() {
        let mut project = fixtures::project();
        let (reply, _answer) = oneshot::channel();

        let applied = serve(
            SessionCommand::Call {
                name: "get_variants".to_owned(),
                input: Value::Null,
                reply,
            },
            &Registry::new(),
            &mut project,
        );

        assert_eq!(applied, Applied { mutated: false });
    }

    #[tokio::test]
    async fn commands_are_serviced_in_the_order_they_arrive() {
        // `Tool::call` takes `&mut Project`, so calls are serialised whatever
        // happens. This asserts they are serialised in the order sent, which
        // is what makes a write-then-read pair from one agent do what it says.
        let session = session();

        session
            .call(
                "write_svg",
                json!({ "source": fixtures::PROJECT.replace("#f05032", "#00ff00") }),
            )
            .await
            .unwrap();

        let after = session.call("get_svg", Value::Null).await.unwrap();
        assert!(
            after.value["source"].as_str().unwrap().contains("#00ff00"),
            "the read did not see the write that preceded it"
        );
    }

    #[tokio::test]
    async fn a_detached_session_can_be_asked_to_save() {
        // `--write`: the agent is the only user, so its edits land on disk.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logo.svg");
        std::fs::write(&path, fixtures::PROJECT).unwrap();

        let session = SessionHandle::detached(Project::open(&path).unwrap(), Registry::new(), true);
        session
            .call(
                "write_svg",
                json!({ "source": fixtures::PROJECT.replace("#f05032", "#00ff00") }),
            )
            .await
            .unwrap();

        // The save happens after the reply is sent, so a read-back proves it
        // landed without racing the write.
        session.call("get_svg", Value::Null).await.unwrap();

        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(on_disk.contains("#00ff00"), "the edit was not saved");
    }
}
