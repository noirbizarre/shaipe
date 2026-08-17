//! A pipe between an agent's subprocess stdio and a running workspace.
//!
//! Deliberately stupid. It parses nothing, frames nothing and knows no
//! protocol, so a change to MCP's wire format cannot break it and it cannot
//! corrupt a message it does not understand. Everything it does is move bytes.
//!
//! ```text
//! agent ──stdio──> shaipe mcp --bridge ──socket──> the workspace's MCP server
//! ```
//!
//! See ADR 010 for why this exists at all.

use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::{Error, Result};

use super::listener::Address;

/// Pipe this process's standard input and output to a workspace.
///
/// Returns when either side closes.
///
/// # Errors
///
/// Returns [`Error::Bridge`] if the workspace cannot be reached, and if the
/// connection fails in a way that is not an ordinary hang-up.
pub async fn run(address: &Address) -> Result<()> {
    match address {
        #[cfg(unix)]
        Address::Unix(path) => {
            let stream = tokio::net::UnixStream::connect(path)
                .await
                .map_err(|source| Error::Bridge {
                    address: address.to_argument(),
                    source,
                })?;
            pump(stream, tokio::io::stdin(), tokio::io::stdout()).await
        }
        Address::Tcp(socket) => {
            let stream = tokio::net::TcpStream::connect(socket)
                .await
                .map_err(|source| Error::Bridge {
                    address: address.to_argument(),
                    source,
                })?;
            pump(stream, tokio::io::stdin(), tokio::io::stdout()).await
        }
    }
}

/// Copy in both directions until either finishes.
///
/// Split out from [`run`] so that the tests can drive it with something other
/// than the process's real standard streams.
async fn pump<S, I, O>(stream: S, input: I, output: O) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
{
    let (mut from_workspace, mut to_workspace) = tokio::io::split(stream);
    let mut input = input;
    let mut output = output;

    let upstream = tokio::io::copy(&mut input, &mut to_workspace);
    let downstream = tokio::io::copy(&mut from_workspace, &mut output);

    // `select!`, not `try_join!`. Either direction finishing means the
    // conversation is over: the agent closing its end of the pipe is the
    // normal way an MCP server is told to exit, and waiting for the *other*
    // direction to notice would leave a bridge process behind for every
    // session. That is a leak nobody would attribute to this file.
    let outcome = tokio::select! {
        result = upstream => result,
        result = downstream => result,
    };

    outcome.map(|_| ()).map_err(|source| Error::Bridge {
        address: "the workspace".to_owned(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use rmcp::ServiceExt as _;
    use tokio::io::AsyncWriteExt as _;

    use super::*;
    use crate::fixtures;
    use crate::mcp::listener::Listener;
    use crate::tools::{Registry, SessionHandle};

    #[tokio::test]
    async fn a_bridge_carries_a_tool_call_to_a_workspace_and_the_answer_back() {
        // The end-to-end proof of the session-scoped design, with no
        // subprocess: a client speaking MCP into a pipe reaches a project that
        // a completely separate task owns, and gets the real answer back.
        let session = SessionHandle::detached(fixtures::project(), Registry::new(), false);
        let listener = Listener::bind(session).await.unwrap();
        let address = listener.address().clone();

        // Stand-ins for the bridge process's own stdin and stdout.
        let (client_side, bridge_side) = tokio::io::duplex(1 << 16);
        let (bridge_in, bridge_out) = tokio::io::split(bridge_side);

        tokio::spawn(async move {
            let stream = tokio::net::UnixStream::connect(match &address {
                #[cfg(unix)]
                Address::Unix(path) => path.clone(),
                #[allow(unreachable_patterns)]
                _ => unreachable!("this test is unix-only"),
            })
            .await
            .unwrap();
            drop(pump(stream, bridge_in, bridge_out).await);
        });

        let client = ().serve(client_side).await.expect("the client connects");

        let tools = client.list_all_tools().await.unwrap();
        assert!(tools.iter().any(|tool| tool.name == "get_variants"));

        let result = client
            .call_tool(rmcp::model::CallToolRequestParams::new("get_variants"))
            .await
            .unwrap();

        let text = format!("{:?}", result.content);
        assert!(text.contains("icon"), "{text}");
        assert!(text.contains("wordmark"), "{text}");
    }

    #[tokio::test]
    async fn a_bridge_pointed_at_nothing_says_so_rather_than_hanging() {
        // What an agent sees if it starts a bridge after the workspace quit.
        // A hang here would leave the agent waiting on a tool list forever.
        let address: Address = "unix:/tmp/shaipe-does-not-exist/mcp.sock".parse().unwrap();

        let error = run(&address).await.unwrap_err();
        assert!(matches!(error, Error::Bridge { .. }), "{error:?}");

        // And it names the address, because the failure is otherwise
        // indistinguishable from every other missing file.
        assert!(error.to_string().contains("shaipe-does-not-exist"));
    }

    #[tokio::test]
    async fn a_bridge_exits_when_the_agent_closes_its_input() {
        // The leak this guards against: an agent that has finished with a
        // server closes its stdin, and a bridge that waited for the *other*
        // direction would stay alive for the life of the workspace. One
        // process per session, never reaped.
        let session = SessionHandle::detached(fixtures::project(), Registry::new(), false);
        let listener = Listener::bind(session).await.unwrap();

        let stream = match listener.address() {
            #[cfg(unix)]
            Address::Unix(path) => tokio::net::UnixStream::connect(path).await.unwrap(),
            #[allow(unreachable_patterns)]
            _ => unreachable!("this test is unix-only"),
        };

        let (mut agent_side, bridge_side) = tokio::io::duplex(1 << 16);
        let (bridge_in, bridge_out) = tokio::io::split(bridge_side);

        let bridge = tokio::spawn(async move { pump(stream, bridge_in, bridge_out).await });

        // The agent hangs up without ever having said anything.
        agent_side.shutdown().await.unwrap();
        drop(agent_side);

        let finished = tokio::time::timeout(std::time::Duration::from_secs(5), bridge).await;
        assert!(
            finished.is_ok(),
            "the bridge outlived the agent that started it"
        );
    }
}
