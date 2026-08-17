//! Where a live workspace serves MCP, and how a bridge finds it.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use tokio::sync::oneshot;

use crate::error::{Error, Result};
use crate::tools::SessionHandle;

use super::Server;

/// Where a bridge should connect to reach a workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    /// A unix socket, in a private directory the workspace owns.
    #[cfg(unix)]
    Unix(PathBuf),
    /// A loopback port.
    ///
    /// The fallback, not the default: any process on the machine can connect
    /// to a loopback port, whereas a unix socket carries filesystem
    /// permissions and can be locked to its owner. Only used where unix
    /// sockets are not available.
    Tcp(SocketAddr),
}

impl Address {
    /// The form passed to `shaipe mcp --bridge`.
    #[must_use]
    pub fn to_argument(&self) -> String {
        match self {
            #[cfg(unix)]
            Self::Unix(path) => format!("unix:{}", path.display()),
            Self::Tcp(address) => format!("tcp:{address}"),
        }
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_argument())
    }
}

impl FromStr for Address {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let invalid = || Error::InvalidAddress {
            value: value.to_owned(),
        };

        match value.split_once(':') {
            #[cfg(unix)]
            Some(("unix", path)) if !path.is_empty() => Ok(Self::Unix(PathBuf::from(path))),
            Some(("tcp", address)) => address.parse().map(Self::Tcp).map_err(|_| invalid()),
            _ => Err(invalid()),
        }
    }
}

/// A socket a live workspace serves MCP on.
///
/// The workspace cannot be an MCP server directly: an ACP agent starts its MCP
/// servers as subprocesses and speaks to them over stdio, and this process's
/// stdio is the terminal the workspace is drawing on. So the server listens
/// here, and a trivial child process pipes one to the other. See ADR 010.
///
/// Dropping this stops accepting new connections and removes the socket.
#[derive(Debug)]
pub struct Listener {
    address: Address,
    /// Dropped to stop the accept loop. The channel is never sent on; closing
    /// it is the signal, so quitting needs no cooperation from the loop.
    _shutdown: oneshot::Sender<()>,
    /// Removed when the workspace quits, taking the socket with it. Held even
    /// on the TCP path, where it is empty, so the field has one type.
    #[cfg(unix)]
    _directory: tempfile::TempDir,
}

impl Listener {
    /// Start accepting connections for a session.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Bridge`] if the socket cannot be created.
    pub async fn bind(session: SessionHandle) -> Result<Self> {
        let (shutdown, stop) = oneshot::channel();

        #[cfg(unix)]
        {
            // A private directory rather than a well-known path: two
            // workspaces open at once must not collide.
            let directory = tempfile::Builder::new()
                .prefix("shaipe-")
                .tempdir()
                .map_err(|source| Error::Bridge {
                    address: "a temporary directory".to_owned(),
                    source,
                })?;

            // Narrowed explicitly, because it is not narrow by default:
            // `tempdir` creates through `mkdir` and the mode is whatever the
            // umask leaves, which on a normal system is 0755. Everything the
            // agent can do to the project goes through this socket, so it is
            // locked to its owner rather than left to the environment.
            // `a_workspace_socket_is_not_reachable_by_other_users` found this.
            std::fs::set_permissions(
                directory.path(),
                std::os::unix::fs::PermissionsExt::from_mode(0o700),
            )
            .map_err(|source| Error::Bridge {
                address: directory.path().display().to_string(),
                source,
            })?;

            let path = directory.path().join("mcp.sock");
            let listener =
                tokio::net::UnixListener::bind(&path).map_err(|source| Error::Bridge {
                    address: path.display().to_string(),
                    source,
                })?;

            // And the socket itself, so the directory is a second line rather
            // than the only one.
            std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))
                .map_err(|source| Error::Bridge {
                    address: path.display().to_string(),
                    source,
                })?;

            let address = Address::Unix(path);
            tokio::spawn(accept_unix(listener, session, stop));

            Ok(Self {
                address,
                _shutdown: shutdown,
                _directory: directory,
            })
        }

        #[cfg(not(unix))]
        {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|source| Error::Bridge {
                    address: "127.0.0.1:0".to_owned(),
                    source,
                })?;
            let address = Address::Tcp(listener.local_addr().map_err(|source| Error::Bridge {
                address: "the listening socket".to_owned(),
                source,
            })?);

            tokio::spawn(accept_tcp(listener, session, stop));

            Ok(Self {
                address,
                _shutdown: shutdown,
            })
        }
    }

    /// Where a bridge should connect.
    #[must_use]
    pub const fn address(&self) -> &Address {
        &self.address
    }
}

/// Serve every connection until the workspace goes away.
///
/// Each gets its own [`Server`] on its own task: an agent may open more than
/// one, and one connection's failure must not take the others with it. They
/// share a [`SessionHandle`], which is what makes them all the same project.
#[cfg(unix)]
async fn accept_unix(
    listener: tokio::net::UnixListener,
    session: SessionHandle,
    mut stop: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            // Closed, not sent on: the workspace signals by dropping.
            _ = &mut stop => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let server = Server::new(session.clone());
                    tokio::spawn(async move {
                        if let Err(error) = server.serve_stream(stream).await {
                            // Logged, never propagated. One agent hanging up
                            // mid-handshake is ordinary, and there is nobody
                            // to return an error to.
                            log::debug!("an MCP connection ended: {error}");
                        }
                    });
                }
                Err(error) => {
                    log::warn!("could not accept an MCP connection: {error}");
                    return;
                }
            },
        }
    }
}

#[cfg(not(unix))]
async fn accept_tcp(
    listener: tokio::net::TcpListener,
    session: SessionHandle,
    mut stop: oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut stop => return,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let server = Server::new(session.clone());
                    tokio::spawn(async move {
                        if let Err(error) = server.serve_stream(stream).await {
                            log::debug!("an MCP connection ended: {error}");
                        }
                    });
                }
                Err(error) => {
                    log::warn!("could not accept an MCP connection: {error}");
                    return;
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn an_address_round_trips_through_its_argument_form() {
        // The argument form is what one process writes and another parses. If
        // these ever disagree, every session-scoped agent silently loses its
        // tools, and the only symptom is an agent that cannot see the project.
        #[cfg(unix)]
        {
            let address = Address::Unix(PathBuf::from("/tmp/shaipe-abc/mcp.sock"));
            assert_eq!(address.to_argument().parse::<Address>().unwrap(), address);
        }

        let address = Address::Tcp("127.0.0.1:8931".parse().unwrap());
        assert_eq!(address.to_argument().parse::<Address>().unwrap(), address);
    }

    #[test]
    fn an_address_that_makes_no_sense_says_so() {
        for value in [
            "",
            "shaipe.sock",
            "unix:",
            "tcp:",
            "tcp:not-a-port",
            "http://x",
        ] {
            let error = value.parse::<Address>().unwrap_err();
            assert!(
                matches!(error, Error::InvalidAddress { .. }),
                "`{value}` was accepted"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_workspace_socket_is_not_reachable_by_other_users() {
        // The reason a unix socket is preferred to a loopback port. A port is
        // open to every process on the machine; this is not.
        use std::os::unix::fs::PermissionsExt as _;

        let (session, _commands) = SessionHandle::channel();
        let listener = Listener::bind(session).await.unwrap();

        let Address::Unix(path) = listener.address() else {
            panic!("a unix build should bind a unix socket");
        };

        let directory = path.parent().expect("the socket lives in a directory");
        let mode = std::fs::metadata(directory).unwrap().permissions().mode();
        assert_eq!(
            mode & 0o077,
            0,
            "the socket directory is group or world accessible"
        );
    }

    #[tokio::test]
    async fn dropping_a_listener_takes_its_socket_with_it() {
        // The workspace quitting must not leave a socket behind that a later
        // bridge could connect to and then hang on.
        let (session, _commands) = SessionHandle::channel();
        let listener = Listener::bind(session).await.unwrap();

        #[cfg(unix)]
        {
            let Address::Unix(path) = listener.address().clone() else {
                panic!("a unix build should bind a unix socket");
            };
            assert!(path.exists());
            drop(listener);
            assert!(!path.exists(), "the socket outlived its workspace");
        }

        #[cfg(not(unix))]
        drop(listener);
    }
}
