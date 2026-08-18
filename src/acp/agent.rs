//! Finding, starting and talking to an agent.

use std::path::{Path, PathBuf};
use std::str::FromStr as _;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, InitializeRequest, McpServer, McpServerStdio,
    NewSessionRequest, PermissionOption, PermissionOptionKind, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption, SessionConfigSelectOption,
    SessionConfigSelectOptions, SessionId, SessionNotification, SetSessionConfigOptionRequest,
    TextContent, ToolKind,
};
use agent_client_protocol::{AcpAgent, ConnectionTo};
use tokio::sync::mpsc;

use std::collections::BTreeMap;

use crate::acp::opencode;
use crate::error::{Error, Result};
use crate::mcp::Address;
use crate::project::Project;

use super::update::{self, AgentUpdate};

/// The agent Shaipe starts when nothing says otherwise.
///
/// OpenCode because it is the agent this was developed and tested against, and
/// overridable because it is not the only one — see [`AgentConfig::command`].
pub const DEFAULT_AGENT: &str = "opencode acp";

/// How much of an update backlog to hold before the agent is made to wait.
const DEPTH: usize = 256;

/// The session modes Shaipe will ask for, best first.
///
/// Agents may open a session in a mode that will not act. OpenCode defaults to
/// `plan`, in which the model deliberately declines to change anything and
/// does not even offer the tools that would — so a user who typed "make the
/// logo blue" would get a plan for making the logo blue and no logo.
///
/// Shaipe is a workspace for *doing* the edit, so it asks for a mode that can.
/// These are OpenCode's names; an agent that offers none of them keeps
/// whatever it opened with, which is why this is a preference and not a
/// requirement.
const WORKING_MODES: [&str; 2] = ["build", "code"];

/// What to answer when the agent asks permission to use one of *its* tools.
///
/// **This is only reachable when the agent asks**, and an agent asks only about
/// what its own configuration marks as needing to. OpenCode's permissions
/// default to `allow`, so against a default install none of this runs and the
/// agent edits whatever it likes. Shaipe cannot prevent that; it notices
/// instead — see ADR 012 and `crate::tui::watch`.
///
/// Shaipe's own tools are a separate matter and are never refused: an MCP call
/// from the session is the user's own workspace acting on the user's own
/// project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    /// Refuse the tools that would change something, allow the rest.
    ///
    /// A deny-list rather than an allow-list, because [`ToolKind::Other`] is
    /// the catch-all every unclassified tool lands in — refusing by default
    /// would block tools nobody meant to block, and a workspace that quietly
    /// breaks an agent's search is worse than one that lets it search.
    #[default]
    Guarded,
    /// Allow everything the agent asks for. `--yes`.
    AllowAll,
    /// Refuse everything, including reads.
    DenyAll,
}

/// The tool kinds [`Policy::Guarded`] refuses.
///
/// Everything that writes. The project is changed through `write_svg`, which
/// validates the document and leaves the file alone until someone saves; a
/// direct write goes around all of it, against a workspace that is holding the
/// same document in memory.
const REFUSED: [ToolKind; 4] = [
    ToolKind::Edit,
    ToolKind::Delete,
    ToolKind::Move,
    ToolKind::Execute,
];

/// One MCP server to hand the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerSpec {
    /// What the agent calls it.
    pub name: String,
    /// The executable.
    pub command: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
}

impl McpServerSpec {
    /// Shaipe's own server, pointed at a live workspace.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AgentSpawn`] if this executable's own path cannot be
    /// determined.
    pub fn shaipe_bridge(address: &Address) -> Result<Self> {
        // `current_exe`, never `"shaipe"`. The workspace the agent has to
        // reach is *this* build; a `shaipe` on `PATH` may be a different
        // version, or absent entirely during development.
        let command = std::env::current_exe().map_err(|source| Error::AgentSpawn {
            command: "shaipe".to_owned(),
            source,
        })?;

        Ok(Self {
            name: "shaipe".to_owned(),
            command,
            args: vec![
                "mcp".to_owned(),
                "--bridge".to_owned(),
                address.to_argument(),
            ],
        })
    }
}

/// How to start an agent.
///
/// A command line, not a provider. Shaipe does not host a model — see ADR 004
/// and ADR 011.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// The command, already split into an argv.
    pub command: Vec<String>,
    /// The directory the agent is given as its working directory.
    pub cwd: PathBuf,
    /// The MCP servers to hand it at `session/new`.
    pub mcp_servers: Vec<McpServerSpec>,
    /// What to do when it asks permission.
    pub policy: Policy,
    /// Environment the agent is started with, on top of this process's own.
    ///
    /// The lever ADR 012 missed: an ACP client cannot restrict an agent
    /// through the protocol, but Shaipe spawns the process and so chooses the
    /// environment it starts in. See [`crate::acp::opencode`] and ADR 013.
    pub env: BTreeMap<String, String>,
    /// Anything about the restriction the user should be told.
    pub note: Option<String>,
}

impl AgentConfig {
    /// The agent for a project, resolved against `PATH`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AgentNotFound`] if the command is not on `PATH`.
    /// Resolved here rather than at spawn so the failure is a diagnostic that
    /// names the command and says how to install it, rather than a bare
    /// "No such file or directory" from inside the protocol crate.
    pub fn discover(command: &str, project: &Project) -> Result<Self> {
        let argv: Vec<String> = command.split_whitespace().map(str::to_owned).collect();

        let program = argv.first().ok_or_else(|| Error::AgentNotFound {
            command: command.to_owned(),
        })?;

        if which(program).is_none() {
            return Err(Error::AgentNotFound {
                command: program.clone(),
            });
        }

        // The project's directory, not the process's: an agent given the wrong
        // root reads the wrong `AGENTS.md` and edits the wrong files.
        //
        // Absolute, because ACP requires it and because `base_directory` of a
        // relative `logo.svg` is `.`, which an agent in a different process
        // resolves against its own working directory. OpenCode given `.`
        // reported its workspace root as `/`.
        let base = project.base_directory();
        let cwd = std::fs::canonicalize(&base).unwrap_or_else(|_| {
            std::env::current_dir().map_or(base, |current| current.join(project.base_directory()))
        });

        // Read from this process's environment rather than replaced, so a
        // configuration somebody already set is added to, not discarded.
        let inherited = std::env::var(opencode::CONFIG).ok();
        let (env, note) = opencode::restrictions(program, inherited.as_deref());
        let note = note.or_else(|| opencode::unrestricted_note(program));

        Ok(Self {
            command: argv,
            cwd,
            mcp_servers: Vec::new(),
            policy: Policy::default(),
            env,
            note,
        })
    }

    /// Choose what to do when the agent asks permission.
    #[must_use]
    pub const fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    /// Add an MCP server for the agent to start.
    #[must_use]
    pub fn with_mcp_server(mut self, server: McpServerSpec) -> Self {
        self.mcp_servers.push(server);
        self
    }

    /// The command line, as written.
    #[must_use]
    pub fn command_line(&self) -> String {
        self.command.join(" ")
    }
}

/// Find an executable on `PATH`.
///
/// Hand-rolled rather than a crate: it is fifteen lines, and the alternative
/// is a dependency for one lookup.
fn which(program: &str) -> Option<PathBuf> {
    // An explicit path is used as given, so `--agent ./my-agent` works.
    if program.contains(std::path::MAIN_SEPARATOR) {
        let path = Path::new(program);
        return path.is_file().then(|| path.to_path_buf());
    }

    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .map(|directory| Path::new(directory).join(program))
        .find(|candidate| candidate.is_file())
}

/// What the workspace should do about an agent.
///
/// Three cases rather than an `Option`, because "there is no agent" and "there
/// was supposed to be one and here is why there is not" are different things
/// to a person looking at a prompt they cannot send.
#[derive(Debug)]
pub enum AgentChoice {
    /// Do not start one. `--no-agent`.
    None,
    /// Start this one.
    Start(Box<AgentConfig>),
    /// One was wanted and could not be had.
    ///
    /// Carries the whole diagnostic, help text included: the prompt pane is
    /// where it will be read, and a workspace that says only "no agent" leaves
    /// someone with nothing to act on.
    Unavailable(String),
}

impl AgentChoice {
    /// Resolve a command line into a choice that never fails.
    ///
    /// Failing to find an agent is not an error the workspace should refuse to
    /// open over — reading your own project has never depended on one.
    #[must_use]
    pub fn discover(command: &str, project: &Project) -> Self {
        match AgentConfig::discover(command, project) {
            Ok(config) => Self::Start(Box::new(config)),
            Err(error) => {
                use miette::Diagnostic as _;

                Self::Unavailable(match error.help() {
                    Some(help) => format!("{error}\n{help}"),
                    None => error.to_string(),
                })
            }
        }
    }
}

/// A turn to send.
struct Turn(String);

/// A running agent session.
#[derive(Debug)]
pub struct Agent {
    prompts: mpsc::Sender<Turn>,
    cancels: mpsc::Sender<()>,
    command: String,
}

impl std::fmt::Debug for Turn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Turn").finish()
    }
}

impl Agent {
    /// Start the agent and begin opening a session.
    ///
    /// **Returns immediately**, before the handshake completes. Success and
    /// failure both arrive on the update channel, as [`AgentUpdate::Ready`]
    /// and [`AgentUpdate::Failed`].
    ///
    /// This is not a stylistic choice; awaiting the handshake deadlocks.
    /// `session/new` carries the MCP server the agent should start, and an
    /// agent starts it and calls `tools/list` on it *before answering*.
    /// Shaipe's MCP server is the live workspace, which can only answer from
    /// its event loop — so a caller that waited here would still be waiting
    /// when the tool list was asked for, the agent would time out, and the
    /// session would come up with none of Shaipe's tools. Verified against
    /// OpenCode 1.18.18, which drops a server that does not answer in time.
    ///
    /// Returning early is also better behaviour: the workspace opens at once
    /// rather than after several seconds of someone looking at a blank
    /// terminal.
    #[must_use]
    pub fn start(config: AgentConfig) -> (Self, mpsc::Receiver<AgentUpdate>) {
        let command = config.command_line();
        let (updates, receiver) = mpsc::channel(DEPTH);
        let (prompts, turns) = mpsc::channel(1);
        let (cancels, cancellations) = mpsc::channel(1);

        tokio::spawn(connect(config, updates, turns, cancellations));

        (
            Self {
                prompts,
                cancels,
                command,
            },
            receiver,
        )
    }

    /// Send a turn.
    ///
    /// Returns as soon as it is queued; everything the agent does in response
    /// arrives on the update receiver.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AgentExited`] if the agent is no longer running.
    pub async fn prompt(&self, text: String) -> Result<()> {
        self.prompts
            .send(Turn(text))
            .await
            .map_err(|_| Error::AgentExited {
                command: self.command.clone(),
            })
    }

    /// Ask the agent to stop the turn it is on.
    ///
    /// # Errors
    ///
    /// Returns [`Error::AgentExited`] if the agent is no longer running.
    pub async fn cancel(&self) -> Result<()> {
        self.cancels.send(()).await.map_err(|_| Error::AgentExited {
            command: self.command.clone(),
        })
    }

    /// The command this agent was started from.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }
}

/// Hold the connection open for the life of the session.
async fn connect(
    config: AgentConfig,
    updates: mpsc::Sender<AgentUpdate>,
    turns: mpsc::Receiver<Turn>,
    cancellations: mpsc::Receiver<()>,
) {
    let command = config.command_line();

    let transport = match spawnable(&command, &config.env) {
        Ok(agent) => agent,
        Err(error) => {
            let failure = Error::AgentSpawn {
                command,
                source: std::io::Error::other(error.to_string()),
            };
            drop(updates.send(AgentUpdate::Failed(failure.to_string())).await);
            return;
        }
    };

    let policy = config.policy;
    let notifier = updates.clone();

    let outcome = agent_client_protocol::Client
        .builder()
        .name("shaipe")
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                // Never `.block_task()` in here: a handler that waits on the
                // connection it is being called from deadlocks it.
                for update in update::from_session(&notification) {
                    // A full channel means the workspace is behind, not that
                    // anything is wrong; dropping the update is better than
                    // stalling the agent's whole read loop.
                    drop(notifier.try_send(update));
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                responder.respond(decide(
                    policy,
                    request.tool_call.fields.kind,
                    request.tool_call.fields.title.as_deref(),
                    &request.options,
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, |cx: ConnectionTo<agent_client_protocol::Agent>| {
            let config = config.clone();
            let command = command.clone();
            let updates = updates.clone();
            let mut turns = turns;
            let mut cancellations = cancellations;

            async move {
                // `block_task` is safe out here — this is the foreground
                // future, not a message handler.
                let initialized = cx
                    .send_request(InitializeRequest::new(ProtocolVersion::V1))
                    .block_task()
                    .await;

                let initialized = match initialized {
                    Ok(response) => response,
                    Err(error) => {
                        fail(&updates, Error::AgentInitialize {
                            command: command.clone(),
                            reason: error.to_string(),
                        }).await;
                        return Ok(());
                    }
                };

                // The spec says an agent that cannot speak the requested
                // version answers with the latest it can, and that a client
                // which cannot speak *that* should close the connection.
                // Every agent shipping today answers 1; this is what stops a
                // future one from being talked to in a dialect it does not
                // understand.
                if initialized.protocol_version != ProtocolVersion::V1 {
                    fail(&updates, Error::AgentInitialize {
                        command: command.clone(),
                        reason: format!(
                            "it speaks protocol version {:?}, and this build speaks version 1",
                            initialized.protocol_version
                        ),
                    }).await;
                    return Ok(());
                }

                let request = NewSessionRequest::new(config.cwd.clone())
                    .mcp_servers(config.mcp_servers.iter().map(to_acp).collect());


                let opened = match cx.send_request(request).block_task().await {
                    Ok(response) => response,
                    Err(error) => {
                        fail(&updates, Error::AgentProtocol {
                            command: command.clone(),
                            reason: format!("it would not open a session: {error}"),
                        }).await;
                        return Ok(());
                    }
                };

                let session = opened.session_id.clone();

                if let Some(request) = working_mode(&session, opened.config_options.as_deref())
                    && let Err(error) = cx.send_request(request).block_task().await
                {
                    // Not fatal. The session works, it just may decline to
                    // edit anything, and saying so beats failing to start.
                    drop(updates.try_send(AgentUpdate::Other(format!(
                        "could not switch the agent out of its default mode: {error}"
                    ))));
                }

                // The workspace is told the agent is usable only now, when a
                // session actually exists and can take a prompt.
                drop(updates.send(AgentUpdate::Ready).await);

                loop {
                    tokio::select! {
                        turn = turns.recv() => {
                            let Some(Turn(text)) = turn else { return Ok(()) };

                            let prompt = PromptRequest::new(
                                session.clone(),
                                vec![ContentBlock::Text(TextContent::new(text))],
                            );

                            match cx.send_request(prompt).block_task().await {
                                Ok(_) => drop(updates.send(AgentUpdate::Idle).await),
                                Err(error) => {
                                    // The turn failed, not the session. Said
                                    // in the transcript and then back to
                                    // waiting, so one bad turn does not end
                                    // the conversation.
                                    drop(updates.send(AgentUpdate::Other(
                                        format!("the turn failed: {error}")
                                    )).await);
                                    drop(updates.send(AgentUpdate::Idle).await);
                                }
                            }
                        }

                        cancelled = cancellations.recv() => {
                            if cancelled.is_none() {
                                return Ok(());
                            }
                            // Not awaited: a notification expects no reply,
                            // and the point of cancelling is that it happens
                            // now rather than after the turn it is cancelling.
                            drop(cx.send_notification(CancelNotification::new(session.clone())));
                        }
                    }
                }
            }
        })
        .await;

    if let Err(error) = outcome {
        // The connection itself ended: the agent exited, or never ran. Said as
        // a proper diagnostic rather than passed through, because the SDK's
        // own message is a JSON blob naming a source file inside the protocol
        // crate — true, and no use at all to someone whose agent will not
        // start.
        log::debug!("the ACP connection ended: {error}");
        fail(&updates, Error::AgentExited { command }).await;
    }
}

/// Ask for a mode that can actually change the project.
///
/// Returns nothing when the agent offers no mode option, or is already in one
/// that works — there is no reason to spend a round trip re-setting it.
fn working_mode(
    session: &SessionId,
    options: Option<&[SessionConfigOption]>,
) -> Option<SetSessionConfigOptionRequest> {
    let option = options?
        .iter()
        .find(|option| option.id.0.as_ref() == "mode")?;

    let SessionConfigKind::Select(select) = &option.kind else {
        // A mode that is not a choice is not one Shaipe can make.
        return None;
    };

    // Already somewhere useful. No reason to spend a round trip.
    if WORKING_MODES.contains(&select.current_value.0.as_ref()) {
        return None;
    }

    // Flattened, because an agent may present its modes in groups and a mode
    // is no less available for being under a heading.
    let choices: Vec<&SessionConfigSelectOption> = match &select.options {
        SessionConfigSelectOptions::Ungrouped(options) => options.iter().collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter())
            .collect(),
        // The protocol may grow another shape. Not knowing how to read it is a
        // reason to leave the mode alone, not to guess at it.
        _ => return None,
    };

    // The first mode this build knows about that the agent actually offers.
    // Never invented: asking for a mode an agent does not have is an error it
    // would be right to refuse.
    let wanted = WORKING_MODES.iter().find_map(|wanted| {
        choices
            .iter()
            .find(|choice| choice.value.0.as_ref() == *wanted)
    })?;

    Some(SetSessionConfigOptionRequest::new(
        session.clone(),
        option.id.clone(),
        wanted.value.clone(),
    ))
}

/// Build the transport, with Shaipe's environment on top of the inherited one.
///
/// `from_str` is what parses a command line; the environment has to be applied
/// to the config it produces, because there is no builder that does both.
fn spawnable(
    command: &str,
    env: &BTreeMap<String, String>,
) -> std::result::Result<AcpAgent, agent_client_protocol::Error> {
    let config = AcpAgent::from_str(command)?.into_config().envs(env);
    Ok(AcpAgent::new(config))
}

/// Tell the workspace the agent is unusable, and why.
///
/// The whole diagnostic, help text included: this is the only place the user
/// will see it, because starting an agent is no longer something that can
/// return an error to the command line.
async fn fail(updates: &mpsc::Sender<AgentUpdate>, error: Error) {
    use miette::Diagnostic as _;

    let text = match error.help() {
        Some(help) => format!("{error}\n{help}"),
        None => error.to_string(),
    };

    drop(updates.send(AgentUpdate::Failed(text)).await);
}

/// Answer a permission request.
///
/// Takes the kind, the title and the options rather than the whole request:
/// they are all the decision uses, and it means a policy can be tested without
/// building a protocol message out of types whose fields are private.
///
/// The title is matched against Shaipe's own tool names, not against a
/// `shaipe_` prefix — the prefix is one agent's namespacing convention, the
/// names are ours.
fn decide(
    policy: Policy,
    kind: Option<ToolKind>,
    title: Option<&str>,
    options: &[PermissionOption],
) -> RequestPermissionResponse {
    let ours = title.is_some_and(is_shaipe_tool);

    let permit = match policy {
        Policy::AllowAll => true,
        // Shaipe's own tools are the *sanctioned* way to change the project.
        // Refusing `write_svg` for being an edit would leave the agent no way
        // to do the one thing the workspace exists to have it do.
        Policy::Guarded => ours || !kind.is_some_and(|kind| REFUSED.contains(&kind)),
        Policy::DenyAll => false,
    };

    // Least-committal first: one call rather than a standing rule, so a
    // decision Shaipe made on the user's behalf does not outlive the turn.
    let wanted: [PermissionOptionKind; 2] = if permit {
        [
            PermissionOptionKind::AllowOnce,
            PermissionOptionKind::AllowAlways,
        ]
    } else {
        [
            PermissionOptionKind::RejectOnce,
            PermissionOptionKind::RejectAlways,
        ]
    };

    let chosen = wanted
        .iter()
        .find_map(|kind| options.iter().find(|option| option.kind == *kind));

    let outcome = match chosen {
        Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
            option.option_id.clone(),
        )),
        // Nothing on offer says what Shaipe means. Cancelling is the honest
        // answer, and better than picking an option whose meaning is a guess.
        //
        // Note this is *not* the same as refusing: an agent reads `Cancelled`
        // as "the user went away" and a rejection as "no, try something else",
        // which is the difference between it giving up and it reaching for
        // `write_svg` instead.
        None => RequestPermissionOutcome::Cancelled,
    };

    RequestPermissionResponse::new(outcome)
}

/// Whether a permission request is about one of Shaipe's own tools.
fn is_shaipe_tool(title: &str) -> bool {
    crate::tools::Registry::new()
        .names()
        .iter()
        .any(|name| title.contains(name.as_str()))
}

/// Turn a Shaipe MCP server description into the protocol's.
fn to_acp(server: &McpServerSpec) -> McpServer {
    let mut stdio = McpServerStdio::new(server.name.clone(), server.command.clone());
    stdio.args = server.args.clone();
    McpServer::Stdio(stdio)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::fixtures;

    #[test]
    fn the_default_agent_is_opencode_over_acp() {
        // Named in one place, because it appears in the CLI's help, in the
        // README and in the error that says how to install it.
        assert_eq!(DEFAULT_AGENT, "opencode acp");
    }

    #[test]
    fn an_agent_that_is_not_installed_names_the_command_and_how_to_get_it() {
        // The first thing anyone without OpenCode will see.
        let error = AgentConfig::discover("definitely-not-installed acp", &fixtures::project())
            .unwrap_err();

        assert!(matches!(error, Error::AgentNotFound { .. }));

        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("definitely-not-installed"), "{rendered}");
        // The help has to say what to do, not only what went wrong.
        assert!(rendered.contains("--no-agent"), "{rendered}");
    }

    #[test]
    fn the_agent_is_given_an_absolute_working_directory() {
        // ACP requires one, and it matters: `base_directory` of a relative
        // `logo.svg` is `.`, which the agent — a different process — resolves
        // against its own working directory. OpenCode given `.` reported its
        // workspace root as `/`, and then read the wrong AGENTS.md.
        let Ok(config) = AgentConfig::discover("sh", &fixtures::project()) else {
            // `sh` is not on PATH. Nothing to assert, and nothing is broken.
            return;
        };

        assert!(
            config.cwd.is_absolute(),
            "the agent was given `{}`, which it will resolve against its own \
             working directory",
            config.cwd.display()
        );
    }

    #[test]
    fn the_bridge_server_points_at_this_build_not_at_whatever_is_on_path() {
        // A `shaipe` on PATH may be a different version, or absent while
        // developing. The agent must reach *this* workspace.
        let address: Address = "unix:/tmp/shaipe-test/mcp.sock".parse().unwrap();
        let spec = McpServerSpec::shaipe_bridge(&address).unwrap();

        assert_eq!(spec.command, std::env::current_exe().unwrap());
        assert_eq!(spec.name, "shaipe");
        assert_eq!(
            spec.args,
            ["mcp", "--bridge", "unix:/tmp/shaipe-test/mcp.sock"]
        );
    }

    #[test]
    fn the_bridge_server_survives_translation_into_the_protocols_shape() {
        // The one place Shaipe's vocabulary meets ACP's. If the arguments are
        // dropped here, the agent starts a bridge pointed at nothing and the
        // only symptom is an agent that cannot see the project.
        let address: Address = "unix:/tmp/shaipe-test/mcp.sock".parse().unwrap();
        let spec = McpServerSpec::shaipe_bridge(&address).unwrap();

        let McpServer::Stdio(stdio) = to_acp(&spec) else {
            panic!(
                "a bridge must be an stdio server; it is the only transport every agent supports"
            );
        };

        assert_eq!(stdio.name, "shaipe");
        assert_eq!(stdio.command, std::env::current_exe().unwrap());
        assert_eq!(
            stdio.args,
            ["mcp", "--bridge", "unix:/tmp/shaipe-test/mcp.sock"]
        );
    }

    #[tokio::test]
    async fn starting_an_agent_does_not_wait_for_it_to_answer() {
        // The regression test for a deadlock that cost a session all of
        // Shaipe's tools without failing anything.
        //
        // `session/new` carries the MCP server the agent should start, and an
        // agent starts it and calls `tools/list` on it *before answering*.
        // Shaipe's MCP server is the live workspace, which can only answer
        // from its event loop. So if `start` waits for the handshake, the
        // workspace never reaches its event loop, the tool list is never
        // answered, the agent times out and drops the server — and everything
        // still "works", minus every tool.
        //
        // `sleep` stands in for an agent that never answers. If `start` ever
        // waits again, this times out.
        let config = AgentConfig {
            command: vec!["sleep".to_owned(), "60".to_owned()],
            cwd: std::env::temp_dir(),
            mcp_servers: Vec::new(),
            policy: Policy::Guarded,
            env: BTreeMap::new(),
            note: None,
        };

        let started = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            Agent::start(config)
        })
        .await;

        assert!(
            started.is_ok(),
            "`Agent::start` waited for the agent; the workspace would deadlock \
             against its own MCP server"
        );
    }

    #[tokio::test]
    async fn an_agent_that_cannot_be_started_reports_it_on_the_update_stream() {
        // Failures cannot be returned any more, so they have to arrive
        // somewhere the workspace will show them. Silently having no agent is
        // the one outcome that would look like a bug in Shaipe.
        let config = AgentConfig {
            command: vec!["/definitely/not/an/agent".to_owned()],
            cwd: std::env::temp_dir(),
            mcp_servers: Vec::new(),
            policy: Policy::Guarded,
            env: BTreeMap::new(),
            note: None,
        };

        let (_agent, mut updates) = Agent::start(config);

        let update = tokio::time::timeout(std::time::Duration::from_secs(10), updates.recv())
            .await
            .expect("a failure should be reported rather than hang");

        assert!(
            matches!(update, Some(AgentUpdate::Failed(_))),
            "expected a failure, got {update:?}"
        );
    }

    #[test]
    fn an_agent_that_is_not_installed_still_lets_the_workspace_open() {
        // Reading your own project has never depended on having an agent, and
        // it does not start now. The reason travels with the choice so the
        // prompt pane can say it.
        let choice = AgentChoice::discover("definitely-not-installed acp", &fixtures::project());

        let AgentChoice::Unavailable(reason) = choice else {
            panic!("a missing agent should be reported, not fatal");
        };

        assert!(reason.contains("definitely-not-installed"), "{reason}");
        // The help, which is the half that says what to do about it.
        assert!(reason.contains("--no-agent"), "{reason}");
    }

    #[test]
    fn an_explicit_path_is_used_as_given() {
        // `--agent ./my-agent` has to work without being on PATH.
        assert!(which("/does/not/exist").is_none());
        assert_eq!(which("/bin/sh"), Some(PathBuf::from("/bin/sh")));
    }

    /// The options an agent offers when it asks to run something.
    fn options() -> Vec<PermissionOption> {
        use agent_client_protocol::schema::v1::PermissionOptionId;

        vec![
            PermissionOption::new(
                PermissionOptionId::new("allow-once"),
                "Allow once",
                PermissionOptionKind::AllowOnce,
            ),
            PermissionOption::new(
                PermissionOptionId::new("allow-always"),
                "Always allow",
                PermissionOptionKind::AllowAlways,
            ),
            PermissionOption::new(
                PermissionOptionId::new("reject-once"),
                "Reject",
                PermissionOptionKind::RejectOnce,
            ),
        ]
    }

    /// Which option a policy picked, by identifier.
    fn chose(policy: Policy, kind: Option<ToolKind>, title: Option<&str>) -> Option<String> {
        match decide(policy, kind, title, &options()).outcome {
            RequestPermissionOutcome::Selected(selected) => Some(selected.option_id.0.to_string()),
            _ => None,
        }
    }

    #[test]
    fn the_agents_own_edit_is_refused() {
        // The project is changed through `write_svg`, which validates the
        // document and leaves the file alone until somebody saves. A direct
        // write goes around all of it.
        for kind in [
            ToolKind::Edit,
            ToolKind::Delete,
            ToolKind::Move,
            ToolKind::Execute,
        ] {
            assert_eq!(
                chose(Policy::Guarded, Some(kind), Some("write")).as_deref(),
                Some("reject-once"),
                "{kind:?} should be refused"
            );
        }
    }

    #[test]
    fn the_agents_own_reads_are_allowed() {
        // An agent that can read `AGENTS.md` and search the tree does better
        // work, and neither can hurt the project.
        for kind in [
            ToolKind::Read,
            ToolKind::Search,
            ToolKind::Fetch,
            ToolKind::Think,
        ] {
            assert_eq!(
                chose(Policy::Guarded, Some(kind), Some("read")).as_deref(),
                Some("allow-once"),
                "{kind:?} should be allowed"
            );
        }
    }

    #[test]
    fn shaipes_own_write_svg_is_never_refused() {
        // It is classified as an edit, because it is one. Refusing it would
        // leave the agent no way to do the one thing the workspace exists to
        // have it do.
        assert_eq!(
            chose(
                Policy::Guarded,
                Some(ToolKind::Edit),
                Some("shaipe_write_svg")
            )
            .as_deref(),
            Some("allow-once")
        );
    }

    #[test]
    fn an_unclassified_tool_is_not_refused_by_accident() {
        // `Other` is the catch-all every tool without a kind lands in. A
        // deny-list keeps those working; an allow-list would have blocked
        // tools nobody meant to block.
        assert_eq!(
            chose(Policy::Guarded, Some(ToolKind::Other), Some("something")).as_deref(),
            Some("allow-once")
        );
        assert_eq!(
            chose(Policy::Guarded, None, None).as_deref(),
            Some("allow-once")
        );
    }

    #[test]
    fn a_refusal_says_no_rather_than_pretending_the_user_left() {
        // An agent reads `Cancelled` as "the user went away" and a rejection
        // as "no, try something else". Only the second makes it reach for
        // `write_svg`.
        let response = decide(
            Policy::Guarded,
            Some(ToolKind::Edit),
            Some("write"),
            &options(),
        );
        assert!(matches!(
            response.outcome,
            RequestPermissionOutcome::Selected(_)
        ));
    }

    #[test]
    fn allowing_everything_allows_an_edit() {
        assert_eq!(
            chose(Policy::AllowAll, Some(ToolKind::Edit), Some("write")).as_deref(),
            Some("allow-once")
        );
    }

    #[test]
    fn refusing_everything_refuses_a_read() {
        assert_eq!(
            chose(Policy::DenyAll, Some(ToolKind::Read), Some("read")).as_deref(),
            Some("reject-once")
        );
    }

    #[test]
    fn a_one_off_answer_is_preferred_to_a_standing_rule() {
        // A decision Shaipe made on the user's behalf should not outlive the
        // turn it was made for.
        assert_eq!(
            chose(Policy::AllowAll, Some(ToolKind::Read), None).as_deref(),
            Some("allow-once")
        );
    }

    #[test]
    fn a_request_with_no_option_that_fits_is_cancelled_rather_than_guessed() {
        // An agent is entitled to offer options none of which mean what Shaipe
        // means. Picking one anyway would be inventing an answer.
        let only_always = vec![PermissionOption::new(
            agent_client_protocol::schema::v1::PermissionOptionId::new("x"),
            "Always allow",
            PermissionOptionKind::AllowAlways,
        )];

        let response = decide(
            Policy::Guarded,
            Some(ToolKind::Edit),
            Some("write"),
            &only_always,
        );
        assert!(matches!(
            response.outcome,
            RequestPermissionOutcome::Cancelled
        ));
    }

    #[test]
    fn a_request_with_no_options_at_all_does_not_panic() {
        for policy in [Policy::Guarded, Policy::AllowAll, Policy::DenyAll] {
            let response = decide(policy, Some(ToolKind::Read), None, &[]);
            assert!(matches!(
                response.outcome,
                RequestPermissionOutcome::Cancelled
            ));
        }
    }
}
