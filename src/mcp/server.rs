//! The MCP server itself.

use base64::Engine as _;
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
    ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServiceExt};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::error::{Error, Result};
use crate::tools::{SessionHandle, ToolDescriptor, ToolOutput};

/// What the agent is told before it reads the tool list.
///
/// Prompt text, and the most valuable sentence in it is the third: a model
/// that has read an SVG believes it knows what the artwork looks like, and it
/// does not. Saying so here is cheaper than letting it find out four turns
/// later.
const PREAMBLE: &str = "\
These tools operate on the Shaipe project the user currently has open — a \
single SVG file that carries the artwork and, in its metadata, the palette, \
variants and render specifications that describe it.

Call `get_project` first, to learn what the project's variants and colours are \
named. You cannot see the artwork by reading the SVG: call `render_svg` to \
look at it, and `render_grid` to check that it still reads when small.

Edit by calling `get_svg`, changing the whole document, and sending it back \
through `write_svg`. Do not edit the project file with a text editor or a \
shell, even though you are able to: a workspace is holding this same document \
open, `write_svg` is what validates your SVG before it replaces anything, and \
writing around it means your change is neither checked nor visible to the \
person watching it.

For a change contained to one variant, prefer `write_variant` over `write_svg`: \
it replaces just that element and is validated the same way, without \
resending the whole document. `set_palette_colour` and `set_generation` edit \
the project's metadata directly and need no document at all.";

/// An MCP server over one Shaipe session.
///
/// Cloned per connection; the [`SessionHandle`] inside is what makes several
/// connections operate on the same project.
#[derive(Debug, Clone)]
pub struct Server {
    session: SessionHandle,
}

impl Server {
    /// Serve one session.
    #[must_use]
    pub const fn new(session: SessionHandle) -> Self {
        Self { session }
    }

    /// Serve over this process's standard input and output.
    ///
    /// # Errors
    ///
    /// Returns [`Error::McpServe`] if the handshake fails or the stream breaks.
    pub async fn serve_stdio(self) -> Result<()> {
        let (read, write) = rmcp::transport::io::stdio();
        self.serve_pair(read, write).await
    }

    /// Serve over an arbitrary byte stream.
    ///
    /// A socket for a live workspace, or an in-process duplex for a test.
    /// MCP does not care, which is the whole reason the bridge can be a pipe.
    ///
    /// # Errors
    ///
    /// Returns [`Error::McpServe`] if the handshake fails or the stream breaks.
    pub async fn serve_stream<S>(self, stream: S) -> Result<()>
    where
        S: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        // Split rather than handed over whole: rmcp has an adapter for a
        // combined stream too, but the pair is the shape a split socket and a
        // pair of standard streams both already have, so one code path covers
        // every transport Shaipe serves.
        let (read, write) = tokio::io::split(stream);
        self.serve_pair(read, write).await
    }

    /// Serve over a reader and a writer that are not the same object.
    ///
    /// # Errors
    ///
    /// Returns [`Error::McpServe`] if the handshake fails or the stream breaks.
    pub async fn serve_pair<R, W>(self, read: R, write: W) -> Result<()>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let transport = (read, write);
        let running = self
            .serve(transport)
            .await
            .map_err(|error| Error::McpServe {
                reason: error.to_string(),
            })?;

        running.waiting().await.map_err(|error| Error::McpServe {
            reason: error.to_string(),
        })?;

        Ok(())
    }
}

/// Turn a Shaipe tool description into the MCP one.
fn advertise(descriptor: &ToolDescriptor) -> Tool {
    let schema = descriptor
        .input_schema
        .as_object()
        .cloned()
        .unwrap_or_default();

    let mut tool = Tool::new(
        descriptor.name,
        descriptor.description,
        std::sync::Arc::new(schema),
    );

    // The read-only hint. Only a hint — the spec is explicit that a client may
    // not rely on it — but it is what lets an agent decide a call is safe to
    // make without asking, which is most of what makes the read tools usable.
    tool.annotations = Some(rmcp::model::ToolAnnotations::from_raw(
        None,
        Some(!descriptor.mutates),
        None,
        None,
        None,
    ));

    tool
}

/// Turn a tool's answer into content blocks.
fn present(output: ToolOutput) -> Vec<ContentBlock> {
    let mut content = Vec::with_capacity(1 + output.images.len() * 2);

    content.push(ContentBlock::text(
        serde_json::to_string_pretty(&output.value).unwrap_or_else(|_| output.value.to_string()),
    ));

    for image in output.images {
        // The label first: a transcript holding five images of the same mark
        // at five sizes has nothing else to say which is which.
        content.push(ContentBlock::text(image.label));
        content.push(ContentBlock::image(
            base64::engine::general_purpose::STANDARD.encode(&image.bytes),
            image.mime_type,
        ));
    }

    content
}

/// Render an error the way the model needs to read it.
///
/// Shaipe's diagnostics carry the help text that says what to do instead, and
/// dropping it here would throw away the most useful half of every error a
/// model is capable of recovering from.
fn explain(error: &Error) -> String {
    use miette::Diagnostic as _;

    let mut text = match error.help() {
        Some(help) => format!("{error}\n\n{help}"),
        None => error.to_string(),
    };

    // The cause travels too. Without it a rejected write says only that the
    // SVG "is not a valid Shaipe project", and a file that could not be read
    // says only which file — never whether it was missing, unreadable or
    // malformed, which are the three things a model would do differently.
    // A cause that is itself one of Shaipe's errors brings its own advice.
    let mut cause = std::error::Error::source(error);
    while let Some(current) = cause {
        text.push_str(&format!("\n\nCaused by: {current}"));
        if let Some(help) = current
            .downcast_ref::<Error>()
            .and_then(miette::Diagnostic::help)
        {
            text.push_str(&format!("\n{help}"));
        }
        cause = current.source();
    }

    text
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        info.server_info = Implementation::new("shaipe", env!("CARGO_PKG_VERSION"));
        // The reconstruction workflow is Shaipe's own text and lives with the
        // workflow model; this adapter only carries it. Sent here rather than
        // in the user's prompt or a chat turn, so it reaches every agent that
        // reads `instructions` and never ends up in the project's metadata.
        info.instructions = Some(format!(
            "{PREAMBLE}\n\n{}",
            crate::workflow::instructions::reconstruction()
        ));
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, ErrorData> {
        // Asked of the session rather than of a local registry, so that a
        // workspace which registered a tool of its own advertises it.
        let descriptors = self
            .session
            .list()
            .await
            .map_err(|error| ErrorData::internal_error(explain(&error), None))?;

        Ok(ListToolsResult::with_all_items(
            descriptors.iter().map(advertise).collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, ErrorData> {
        let input = request
            .arguments
            .map_or(serde_json::Value::Null, serde_json::Value::Object);

        match self.session.call(&request.name, input).await {
            Ok(output) => Ok(CallToolResult::success(present(output)).into()),

            // The session having gone is the one failure that is genuinely
            // about the transport: there is nothing on the other end any more,
            // and no argument the model could change would help.
            Err(error @ Error::SessionClosed) => {
                Err(ErrorData::internal_error(explain(&error), None))
            }

            // Everything else comes back as a *result*. A model that asked for
            // a variant that does not exist needs to read which ones do and
            // try again; a protocol error would be opaque to it and would look
            // to the agent like Shaipe was broken.
            Err(error) => {
                Ok(CallToolResult::error(vec![ContentBlock::text(explain(&error))]).into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rmcp::ServiceExt;
    use serde_json::json;

    use super::*;
    use crate::fixtures;
    use crate::tools::Registry;

    /// A real client talking to a real server over an in-process duplex.
    ///
    /// No subprocess and no socket: this exercises the handler, the schemas
    /// and the content encoding, which is where protocol bugs actually live.
    /// The transports are covered separately, by the bridge's own tests.
    async fn client() -> rmcp::service::RunningService<rmcp::RoleClient, ()> {
        let (server_side, client_side) = tokio::io::duplex(1 << 16);

        let session = SessionHandle::detached(fixtures::project(), Registry::new(), false);
        tokio::spawn(async move {
            drop(Server::new(session).serve_stream(server_side).await);
        });

        ().serve(client_side).await.expect("the client connects")
    }

    #[tokio::test]
    async fn an_mcp_client_sees_every_tool_the_registry_holds() {
        let client = client().await;
        let mut names: Vec<_> = client
            .list_all_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        names.sort();

        assert_eq!(names, Registry::new().names());
    }

    #[tokio::test]
    async fn every_advertised_schema_survives_the_wire_as_a_json_schema_object() {
        let client = client().await;
        for tool in client.list_all_tools().await.unwrap() {
            assert_eq!(
                tool.input_schema.get("type").and_then(|t| t.as_str()),
                Some("object"),
                "`{}` did not advertise an object schema",
                tool.name
            );
            assert!(
                tool.input_schema.contains_key("properties"),
                "`{}` did not advertise its properties",
                tool.name
            );
        }
    }

    #[tokio::test]
    async fn the_read_only_tools_say_so_in_their_annotations() {
        // What lets an agent call `render_svg` without stopping to ask.
        // Checked against the registry's own `mutates()`, not a hardcoded
        // name, so this keeps catching a disagreement as the mutating set
        // grows rather than only while `write_svg` was the sole one.
        let registry = Registry::new();
        let client = client().await;
        for tool in client.list_all_tools().await.unwrap() {
            let read_only = tool
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.read_only_hint);
            let mutates = registry
                .get(&tool.name)
                .unwrap_or_else(|| panic!("`{}` is not in the registry", tool.name))
                .mutates();
            assert_eq!(
                read_only,
                Some(!mutates),
                "`{}` has the wrong read-only hint",
                tool.name
            );
        }
    }

    #[tokio::test]
    async fn rendering_through_mcp_returns_an_image_block_a_model_can_see() {
        // The test that proves the product claim. Everything else in this
        // crate is in service of a model being able to look at the artwork,
        // and this is the assertion that it actually arrives as an image
        // rather than as a base64 string in a JSON field.
        let client = client().await;
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("render_svg").with_arguments(
                    json!({ "variant": "icon", "width": 64 })
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();

        assert_eq!(result.is_error, Some(false));

        let images: Vec<_> = result
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Image(image) => Some(image),
                _ => None,
            })
            .collect();

        let [image] = images[..] else {
            panic!("expected exactly one image block, got {}", images.len());
        };
        assert_eq!(image.mime_type, "image/png");

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&image.data)
            .expect("the image block is valid base64");
        let pixmap = tiny_skia::Pixmap::decode_png(&bytes).expect("it is a PNG");
        assert_eq!((pixmap.width(), pixmap.height()), (64, 64));
    }

    #[tokio::test]
    async fn render_grid_through_mcp_returns_one_image_block_per_size() {
        let client = client().await;
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("render_grid").with_arguments(
                    json!({ "variant": "icon", "sizes": [64, 32, 16] })
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();

        let sizes: Vec<_> = result
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Image(image) => {
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(&image.data)
                        .unwrap();
                    let pixmap = tiny_skia::Pixmap::decode_png(&bytes).unwrap();
                    Some(pixmap.width())
                }
                _ => None,
            })
            .collect();

        // Each at its own resolution: the point of the tool, asserted after a
        // round trip rather than only in the tool's own tests.
        assert_eq!(sizes, [64, 32, 16]);
    }

    #[tokio::test]
    async fn a_tool_error_comes_back_as_a_readable_result_rather_than_a_protocol_failure() {
        // A model that guessed a variant name has to be able to read which
        // ones exist and try again. A protocol error would be opaque to it,
        // and would look to the agent like Shaipe itself was broken.
        let client = client().await;
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("render_svg").with_arguments(
                    json!({ "variant": "watermark" })
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .expect("a tool failure is not a protocol failure");

        assert_eq!(result.is_error, Some(true));

        let text = result
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<String>();

        assert!(text.contains("watermark"), "{text}");
        // The help, which is the half that says what to do instead.
        assert!(text.contains("icon"), "{text}");
        assert!(text.contains("wordmark"), "{text}");
    }

    #[tokio::test]
    async fn writing_an_invalid_svg_through_mcp_leaves_the_project_untouched() {
        let client = client().await;

        let before = client
            .call_tool(rmcp::model::CallToolRequestParams::new("get_svg"))
            .await
            .unwrap();

        let failed = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("write_svg").with_arguments(
                    json!({ "source": "<svg>truncated" })
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert_eq!(failed.is_error, Some(true));

        let after = client
            .call_tool(rmcp::model::CallToolRequestParams::new("get_svg"))
            .await
            .unwrap();

        assert_eq!(before.content, after.content);
    }

    #[tokio::test]
    async fn a_rejected_write_says_why_it_was_rejected() {
        // "Not a valid Shaipe project" alone leaves a model to guess between
        // a truncated document, a lost metadata block and a bad version, and
        // the fixes differ. The cause is the error's source, and only the
        // adapter can put it on the wire.
        let client = client().await;
        let result = client
            .call_tool(
                rmcp::model::CallToolRequestParams::new("write_svg").with_arguments(
                    json!({
                        "source": "<svg xmlns=\"http://www.w3.org/2000/svg\"><circle r=\"5\"/></svg>"
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                ),
            )
            .await
            .unwrap();

        assert_eq!(result.is_error, Some(true));
        let text = result
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<String>();

        assert!(text.contains("not a valid Shaipe project"), "{text}");
        assert!(text.contains("Caused by:"), "{text}");
        assert!(text.to_lowercase().contains("metadata"), "{text}");
    }

    #[test]
    fn an_error_with_no_cause_is_explained_without_one() {
        let text = explain(&Error::UnknownTool {
            tool: "vectorise".to_owned(),
            known: vec!["render_svg".to_owned()],
        });

        assert!(!text.contains("Caused by:"), "{text}");
    }

    #[test]
    fn a_file_that_could_not_be_read_says_whether_it_was_missing() {
        let text = explain(&Error::io(
            "mockup.png",
            std::io::Error::from(std::io::ErrorKind::NotFound),
        ));

        assert!(text.contains("mockup.png"), "{text}");
        assert!(text.contains("Caused by:"), "{text}");
    }

    #[tokio::test]
    async fn an_unknown_tool_lists_the_ones_that_exist() {
        let client = client().await;
        let result = client
            .call_tool(rmcp::model::CallToolRequestParams::new("vectorise"))
            .await
            .unwrap();

        assert_eq!(result.is_error, Some(true));
        let text = format!("{:?}", result.content);
        assert!(text.contains("render_svg"), "{text}");
    }

    #[tokio::test]
    async fn the_server_tells_the_model_it_cannot_see_the_artwork_by_reading_it() {
        // The preamble is prompt text, and this sentence is the one that stops
        // a model from believing it has inspected a logo because it has parsed
        // the SVG.
        let client = client().await;
        let instructions = client
            .peer_info()
            .and_then(|info| info.instructions.clone())
            .expect("the server introduces itself");

        assert!(instructions.contains("render_svg"), "{instructions}");
        assert!(instructions.contains("cannot see"), "{instructions}");
    }

    #[tokio::test]
    async fn the_server_tells_the_agent_how_to_reconstruct_from_a_reference() {
        // Without this an agent handed a reference has no reason to measure it
        // or compare against it, and the tools for doing so go unused.
        let client = client().await;
        let instructions = client
            .peer_info()
            .and_then(|info| info.instructions.clone())
            .expect("the server introduces itself");

        for needle in [
            "get_workflow",
            "get_reference_analysis",
            "compare_reference",
        ] {
            assert!(
                instructions.contains(needle),
                "{needle} missing: {instructions}"
            );
        }
        // The mechanics preamble is kept alongside, not replaced by it.
        assert!(instructions.contains("write_svg"), "{instructions}");
    }
}
