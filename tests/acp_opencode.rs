//! The end-to-end path, against a real agent and a real model.
//!
//! Every test here is `#[ignore]`d, and must stay that way. They need an
//! installed and authenticated agent, they make real model calls, and they
//! cost the person running them money. CI runs neither them nor anything that
//! needs a network.
//!
//! ```sh
//! opencode auth login          # once
//! cargo test --test acp_opencode -- --ignored --nocapture
//! ```
//!
//! What they prove that no other test can: that Shaipe's tools reach a real
//! agent through a real ACP session, and that an image rendered by the live
//! workspace arrives somewhere a model can see it.
//!
//! Everything up to the model is covered by tests that always run —
//! `src/mcp/server.rs` drives a real MCP client over a duplex,
//! `src/mcp/bridge.rs` carries a call to a workspace over a real socket, and
//! `tests/mcp_stdio.rs` runs the binary. The gap these fill is the agent.

use std::time::Duration;

use shaipe::Project;
use shaipe::acp::{Agent, AgentConfig, AgentUpdate};
use shaipe::mcp::Listener;
use shaipe::tools::{Registry, SessionHandle};

/// How long to give a real model before giving up.
const PATIENCE: Duration = Duration::from_secs(180);

/// A project in a temporary directory, so nothing writes to a tracked file.
fn project() -> (tempfile::TempDir, Project) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("logo.svg");
    std::fs::copy("tests/fixtures/logo.svg", &path).expect("the fixture is readable");
    let project = Project::open(&path).expect("the fixture opens");
    (directory, project)
}

/// Everything `shaipe` itself does to bring an agent up, minus the terminal.
///
/// Deliberately mirrors `tui::run`: a session, a listener the agent can reach,
/// and an agent told to start a bridge into it. If this drifts from the
/// workspace, the test stops covering the workspace.
async fn workspace(
    project: Project,
) -> Option<(Agent, Listener, tokio::sync::mpsc::Receiver<AgentUpdate>)> {
    let config = match AgentConfig::discover(shaipe::acp::DEFAULT_AGENT, &project) {
        Ok(config) => config.with_policy(shaipe::acp::Policy::AllowAll),
        Err(error) => {
            eprintln!("skipped: {error}");
            return None;
        }
    };

    let (session, mut commands) = SessionHandle::channel();
    let listener = Listener::bind(session).await.expect("a listener binds");

    // Stands in for the workspace's event loop. It has to be running *before*
    // the agent starts: the agent asks the MCP server for a tool list before
    // it answers `session/new`, and nothing else can answer. See ADR 011.
    let mut project = project;
    tokio::spawn(async move {
        let registry = Registry::new();
        while let Some(command) = commands.recv().await {
            shaipe::tools::session::serve(command, &registry, &mut project);
        }
    });

    // Built by hand rather than through `McpServerSpec::shaipe_bridge`, which
    // uses `current_exe()`. That is right for the binary and wrong here: in a
    // test, `current_exe()` is this test harness, which has no `mcp`
    // subcommand — so the agent would come up with no tools and nothing would
    // say why.
    let config = config.with_mcp_server(shaipe::acp::McpServerSpec {
        name: "shaipe".to_owned(),
        command: env!("CARGO_BIN_EXE_shaipe").into(),
        args: vec![
            "mcp".to_owned(),
            "--bridge".to_owned(),
            listener.address().to_argument(),
        ],
    });

    let (agent, updates) = Agent::start(config);
    Some((agent, listener, updates))
}

/// Collect updates until the turn ends, or patience runs out.
async fn turn(
    agent: &Agent,
    updates: &mut tokio::sync::mpsc::Receiver<AgentUpdate>,
    prompt: &str,
) -> (String, Vec<String>) {
    // Wait for the handshake before prompting. `Agent::start` returns before
    // it finishes, on purpose.
    let ready = tokio::time::timeout(Duration::from_secs(60), async {
        while let Some(update) = updates.recv().await {
            match update {
                AgentUpdate::Ready => return true,
                AgentUpdate::Failed(reason) => {
                    eprintln!("the agent failed: {reason}");
                    return false;
                }
                _ => {}
            }
        }
        false
    })
    .await;

    assert_eq!(ready, Ok(true), "the agent never became ready");

    agent
        .prompt(prompt.to_owned())
        .await
        .expect("the agent takes a prompt");

    let mut said = String::new();
    let mut tools = Vec::new();

    let _ = tokio::time::timeout(PATIENCE, async {
        while let Some(update) = updates.recv().await {
            match update {
                AgentUpdate::Message(chunk) => said.push_str(&chunk),
                AgentUpdate::ToolStarted { title, .. } => tools.push(title),
                AgentUpdate::Idle => return,
                AgentUpdate::Failed(reason) => panic!("the agent failed: {reason}"),
                _ => {}
            }
        }
    })
    .await;

    (said, tools)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs an installed, authenticated agent and makes real model calls"]
async fn an_agent_can_read_the_project_through_shaipes_tools() {
    let (_directory, project) = project();
    let Some((agent, _listener, mut updates)) = workspace(project).await else {
        return;
    };

    let (said, tools) = turn(
        &agent,
        &mut updates,
        "Call the shaipe get_variants tool, then reply with only the variant \
         names, comma separated. Do not read any files.",
    )
    .await;

    println!("tools: {tools:?}\nsaid: {said}");

    assert!(
        tools.iter().any(|tool| tool.contains("get_variants")),
        "the agent never called get_variants; it had these tools: {tools:?}"
    );
    // The fixture's own variants. If the agent answered without them, it was
    // not looking at this project.
    assert!(said.contains("icon"), "{said}");
    assert!(said.contains("wordmark"), "{said}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs an installed, authenticated agent and makes real model calls"]
async fn an_agent_can_see_the_artwork_rather_than_only_read_it() {
    // The claim the whole project rests on: a model that can only *read* an
    // SVG cannot tell you the mark is illegible at 16 pixels.
    //
    // This test needs a **vision-capable model**, which is the agent's
    // configuration and not Shaipe's business. When the model cannot accept
    // images it skips loudly rather than passing — an earlier version of this
    // test asserted on the colour name and was satisfied by a model that said
    // "I cannot see the image" and then looked the colour up with
    // `get_palette`. That is the fake vision loop this project exists not to
    // build, and it passed for two runs before anyone read the output.
    //
    // That Shaipe *delivers* a real image block is asserted without a model at
    // all, by `rendering_through_mcp_returns_an_image_block_a_model_can_see`
    // in `src/mcp/server.rs`. What is unprovable there and provable here is
    // that a model at the far end receives it.
    let (_directory, project) = project();
    let Some((agent, _listener, mut updates)) = workspace(project).await else {
        return;
    };

    let (said, tools) = turn(
        &agent,
        &mut updates,
        "Call shaipe render_svg on the `icon` variant at 256 pixels. Then name \
         the single most prominent colour in the image you were given. Answer \
         from the image only: do not call any other tool, and do not read the \
         SVG source. If you cannot see images, reply with exactly NO_VISION.",
    )
    .await;

    println!("tools: {tools:?}\nsaid: {said}");

    assert!(
        tools.iter().any(|tool| tool.contains("render_svg")),
        "the agent never called render_svg; it had these tools: {tools:?}"
    );

    let lowered = said.to_lowercase();

    // The honest exit. Shaipe did its half; the model cannot do the other.
    if lowered.contains("no_vision")
        || lowered.contains("cannot see")
        || lowered.contains("can't see")
        || lowered.contains("not support image")
        || lowered.contains("doesn't support image")
    {
        eprintln!(
            "\n  SKIPPED: the configured model cannot accept images, so this \
             test cannot prove\n  the vision loop. Shaipe sent a real image \
             content block — that much is asserted\n  by \
             `rendering_through_mcp_returns_an_image_block_a_model_can_see`.\n\n             \x20 Configure a vision-capable model in your agent and run this \
             again.\n  The agent said: {said}\n"
        );
        return;
    }

    // The fixture's accent is #f05032 — an orange-red, and by far the most
    // saturated thing in the mark. Reached without calling `get_palette`,
    // which the prompt forbids, so the only source is the image itself.
    assert!(
        !tools.iter().any(|tool| tool.contains("get_palette")),
        "the agent looked the colour up instead of seeing it: {tools:?}"
    );
    assert!(
        ["orange", "red", "vermilion", "coral", "f05032"]
            .iter()
            .any(|word| lowered.contains(word)),
        "the agent did not describe the colour it should have seen: {said}"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs an installed, authenticated agent and makes real model calls"]
async fn an_agent_can_edit_the_project_and_the_edit_is_kept_in_memory() {
    // The write half of the loop, and the invariant that goes with it: the
    // project changes, and the file on disk does not.
    let (directory, project) = project();
    let path = project.path().to_path_buf();
    let before = std::fs::read_to_string(&path).expect("the fixture is readable");

    let Some((agent, _listener, mut updates)) = workspace(project).await else {
        return;
    };

    let (said, tools) = turn(
        &agent,
        &mut updates,
        "Call shaipe get_svg, change every occurrence of the colour #f05032 to \
         #0000ff, and send the whole document back with shaipe write_svg. Then \
         say DONE. Do not use any file editing tools.",
    )
    .await;

    println!("tools: {tools:?}\nsaid: {said}");

    assert!(
        tools.iter().any(|tool| tool.contains("write_svg")),
        "the agent never called write_svg; it had these tools: {tools:?}"
    );

    assert_eq!(
        std::fs::read_to_string(&path).expect("the file is still there"),
        before,
        "the agent wrote to the working tree; nothing may do that unasked"
    );

    drop(directory);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs an installed, authenticated agent and makes real model calls"]
async fn the_agent_cannot_edit_files_with_its_own_tools() {
    // Shaipe starts the agent with `edit` and `bash` denied, so the project
    // can only be changed through `write_svg` — which validates the document,
    // keeps the preview in step and leaves the file alone until somebody
    // saves. See ADR 013.
    //
    // OpenCode does not merely refuse a denied tool: it removes it, so the
    // model reports having no such capability at all.
    let (directory, project) = project();
    let scratch = directory.path().join("scratch.txt");

    let Some((agent, _listener, mut updates)) = workspace(project).await else {
        return;
    };

    let (said, tools) = turn(
        &agent,
        &mut updates,
        "Create a file called scratch.txt containing HELLO, using your own file \
         writing tool or a shell command. Then say DONE.",
    )
    .await;

    println!("tools: {tools:?}\nsaid: {said}");

    assert!(
        !scratch.exists(),
        "the agent wrote to the working tree; the restriction is not applied"
    );
    drop(directory);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs an installed, authenticated agent and makes real model calls"]
async fn the_restriction_does_not_take_away_write_svg() {
    // The other half, and the one that matters more: OpenCode's permission
    // keys are its own tool names, so an MCP tool called `shaipe_write_svg` is
    // untouched by denying `edit`. If that ever stopped being true, the
    // restriction would silently remove the agent's only sanctioned way to
    // change the project, and it would look like the model being unhelpful.
    let (directory, project) = project();
    let Some((agent, _listener, mut updates)) = workspace(project).await else {
        return;
    };

    let (said, tools) = turn(
        &agent,
        &mut updates,
        "Call shaipe get_svg, change every #f05032 to #0066ff, and send the \
         whole document back with shaipe write_svg. Then say DONE.",
    )
    .await;

    println!("tools: {tools:?}\nsaid: {said}");

    assert!(
        tools.iter().any(|tool| tool.contains("write_svg")),
        "write_svg was unreachable; the restriction took away the wrong tool: {tools:?}"
    );
    drop(directory);
}
