//! The error type, and the diagnostics it renders to.
//!
//! `thiserror` defines them, `miette` renders them. A diagnostic must carry the
//! two things the user does not already know: what specifically failed, and
//! what to do about it.

use std::path::PathBuf;

use miette::Diagnostic;
use thiserror::Error;

/// The crate's result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong.
///
/// Diagnostic codes are `shaipe::<module>::<kind>`. A code is a public
/// identifier users grep for, so renaming one is a breaking change.
#[derive(Debug, Error, Diagnostic)]
#[non_exhaustive]
pub enum Error {
    /// Reading or writing a file failed.
    #[error("failed to access `{}`", path.display())]
    #[diagnostic(code(shaipe::error::io))]
    Io {
        /// The path that could not be accessed.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },

    /// The file is not well-formed XML, so it is not an SVG either.
    #[error("`{}` is not well-formed XML", path.display())]
    #[diagnostic(
        code(shaipe::project::malformed_xml),
        help("A Shaipe project is an SVG document. Open the file and check it parses.")
    )]
    MalformedXml {
        /// The offending file.
        path: PathBuf,
        /// Why.
        #[source]
        source: roxmltree::Error,
    },

    /// The document's root element is not `<svg>`.
    #[error("`{}` is not an SVG document", path.display())]
    #[diagnostic(
        code(shaipe::project::not_svg),
        help(
            "The root element must be `<svg>` in the `{}` namespace.",
            crate::project::SVG_NAMESPACE
        )
    )]
    NotSvg {
        /// The offending file.
        path: PathBuf,
    },

    /// The document carries no `<shaipe:project>` metadata.
    #[error("`{}` carries no Shaipe metadata", path.display())]
    #[diagnostic(
        code(shaipe::project::no_metadata),
        help(
            "Add a `<shaipe:project version=\"1\">` element inside `<metadata>`, \
             declaring the `{}` namespace.",
            crate::project::metadata::NAMESPACE
        )
    )]
    NoMetadata {
        /// The offending file.
        path: PathBuf,
    },

    /// The metadata declares a schema version this build does not understand.
    #[error("`{}` uses Shaipe metadata schema version {found}", path.display())]
    #[diagnostic(
        code(shaipe::project::unsupported_schema),
        help("This build understands version {supported}. Upgrade Shaipe.")
    )]
    UnsupportedSchema {
        /// The offending file.
        path: PathBuf,
        /// What the document asked for.
        found: String,
        /// What this build implements.
        supported: u32,
    },

    /// A metadata element is present but malformed.
    #[error("invalid Shaipe metadata in `{}`: {reason}", path.display())]
    #[diagnostic(code(shaipe::project::invalid_metadata))]
    InvalidMetadata {
        /// The offending file.
        path: PathBuf,
        /// What specifically is wrong.
        reason: String,
    },

    /// `usvg` refused the document.
    #[error("failed to parse the SVG in `{}`", path.display())]
    #[diagnostic(code(shaipe::render::parse))]
    ParseSvg {
        /// The offending file.
        path: PathBuf,
        /// Why.
        #[source]
        source: usvg::Error,
    },

    /// A render specification names a variant the project does not declare.
    #[error("unknown variant `{variant}`")]
    #[diagnostic(
        code(shaipe::project::unknown_variant),
        help("The project declares: {}", if known.is_empty() { "no variants at all".to_owned() } else { known.join(", ") })
    )]
    UnknownVariant {
        /// What was asked for.
        variant: String,
        /// What exists.
        known: Vec<String>,
    },

    /// A variant points at an element id that is not in the rendered tree.
    #[error("variant `{variant}` references element `#{element}`, which the SVG does not render")]
    #[diagnostic(
        code(shaipe::render::unknown_element),
        help(
            "Every variant must reference an element that exists and draws something. \
             Check that `#{element}` is present and not empty."
        )
    )]
    UnknownElement {
        /// The variant that pointed nowhere.
        variant: String,
        /// The id it pointed at.
        element: String,
    },

    /// A render specification names a spec the project does not declare.
    #[error("unknown render specification `{spec}`")]
    #[diagnostic(
        code(shaipe::project::unknown_spec),
        help("The project declares: {}", if known.is_empty() { "no render specifications at all".to_owned() } else { known.join(", ") })
    )]
    UnknownSpec {
        /// What was asked for.
        spec: String,
        /// What exists.
        known: Vec<String>,
    },

    /// The project declares no render specifications and none were given.
    #[error("`{}` declares no render specifications", path.display())]
    #[diagnostic(
        code(shaipe::render::nothing_to_render),
        help(
            "Add a `<shaipe:render>` element to the project metadata, or pass \
             `--variant` and `--width` to render one asset ad hoc."
        )
    )]
    NothingToRender {
        /// The offending file.
        path: PathBuf,
    },

    /// A colour literal could not be parsed.
    #[error("`{value}` is not a colour")]
    #[diagnostic(
        code(shaipe::palette::invalid_colour),
        help("Expected `#rgb`, `#rgba`, `#rrggbb` or `#rrggbbaa`.")
    )]
    InvalidColour {
        /// The literal that could not be parsed.
        value: String,
    },

    /// A render specification asked for an impossible canvas.
    #[error("cannot render a {width}x{height} canvas")]
    #[diagnostic(
        code(shaipe::render::invalid_size),
        help("Width and height must both be at least 1 pixel.")
    )]
    InvalidSize {
        /// The requested width.
        width: u32,
        /// The requested height.
        height: u32,
    },

    /// Rasterisation produced nothing.
    #[error("rendering `{spec}` produced an empty image")]
    #[diagnostic(
        code(shaipe::render::empty),
        help("The variant's geometry is probably empty or entirely outside its viewBox.")
    )]
    EmptyRender {
        /// The specification that rendered nothing.
        spec: String,
    },

    /// Encoding the rasterised pixmap failed.
    #[error("failed to encode `{spec}` as PNG")]
    #[diagnostic(code(shaipe::render::encode))]
    Encode {
        /// The specification being encoded.
        spec: String,
        /// Why.
        #[source]
        source: png::EncodingError,
    },

    /// A declared font file could not be loaded.
    #[error("failed to load font `{family}` from `{}`", path.display())]
    #[diagnostic(
        code(shaipe::render::font),
        help(
            "The `src` of a `<shaipe:font>` is resolved relative to the project file. \
             Commit the font alongside the project so CI renders the same glyphs."
        )
    )]
    Font {
        /// The family the file was meant to provide.
        family: String,
        /// Where it was looked for.
        path: PathBuf,
        /// Why.
        #[source]
        source: std::io::Error,
    },

    /// A declared font family was not resolved and system fallback was refused.
    #[error("font family `{family}` is not available")]
    #[diagnostic(
        code(shaipe::render::strict_fonts),
        help(
            "`--strict-fonts` forbids falling back to system fonts, because a system \
             font makes the render depend on the machine. Declare the family with a \
             `<shaipe:font src=\"...\">` pointing at a committed font file."
        )
    )]
    StrictFonts {
        /// The family that could not be resolved from declared files.
        family: String,
    },

    /// Something that should have been a PNG was not.
    #[error("not a PNG")]
    #[diagnostic(
        code(shaipe::preview::not_a_png),
        help(
            "Only raster renders carry pixels. An SVG render has none until something rasterises it."
        )
    )]
    NotAPng,

    /// No external editor could be found to hand the prompt to.
    ///
    /// The message carries the advice rather than leaving it all to `help`,
    /// because the workspace shows this one on its status line — an editor
    /// that cannot be opened must not close a workspace holding unsaved work —
    /// and a status line is one line of `Display` with no room for a
    /// diagnostic.
    #[error("no editor is configured; set $VISUAL or $EDITOR")]
    #[diagnostic(
        code(shaipe::tui::no_editor),
        help("Set $VISUAL or $EDITOR, or edit the prompt in place with `enter`.")
    )]
    NoEditor,

    /// A tool was invoked that the registry does not know.
    #[error("unknown tool `{tool}`")]
    #[diagnostic(
        code(shaipe::tools::unknown),
        help("Known tools: {}", known.join(", "))
    )]
    UnknownTool {
        /// What was asked for.
        tool: String,
        /// What exists.
        known: Vec<String>,
    },

    /// A tool was invoked with arguments it could not accept.
    #[error("invalid arguments for tool `{tool}`: {reason}")]
    #[diagnostic(code(shaipe::tools::invalid_input))]
    InvalidToolInput {
        /// The tool that refused.
        tool: String,
        /// What specifically is wrong.
        reason: String,
    },

    /// A tool was handed a document that is not a usable Shaipe project.
    ///
    /// Its own variant rather than passing the inner error through, because
    /// the model needs to be told two things the inner error cannot say: that
    /// nothing was changed, and how to produce a document that would be
    /// accepted.
    #[error("`{tool}` was given an SVG that is not a valid Shaipe project")]
    #[diagnostic(
        code(shaipe::tools::invalid_svg),
        help(
            "Nothing was changed; the project is exactly as it was. Call \
             `get_svg` for the current document, apply the edit to the whole \
             of it — `<metadata>` included — and send all of it back."
        )
    )]
    InvalidSvgFromTool {
        /// The tool that refused.
        tool: String,
        /// Why the document was rejected.
        ///
        /// Boxed to keep `Error` a sensible size: without it, every `Result`
        /// in the crate grows to hold a nested copy of the whole enum.
        ///
        /// `#[source]` and not `#[diagnostic_source]`: miette wants the latter
        /// to borrow as `dyn Diagnostic`, which a `Box<Error>` does not, and
        /// the cause chain renders either way.
        #[source]
        source: Box<Error>,
    },

    /// A session was addressed after whoever owned its project had gone.
    #[error("the workspace this session belongs to has closed")]
    #[diagnostic(
        code(shaipe::tools::session_closed),
        help(
            "The project's tools are served by a running `shaipe` workspace. \
             Reopen it, or run `shaipe mcp <project>` for a session that \
             stands on its own."
        )
    )]
    SessionClosed,

    /// The MCP server would not start, or stopped badly.
    #[error("the MCP server failed: {reason}")]
    #[diagnostic(
        code(shaipe::mcp::serve),
        help(
            "`shaipe mcp` speaks JSON-RPC on standard output and nothing else. \
             A stray print or log line there corrupts the stream, so check \
             nothing else in the pipeline is writing to it."
        )
    )]
    McpServe {
        /// What the transport reported.
        reason: String,
    },

    /// A bridge could not reach the workspace it was started for.
    #[error("cannot reach the workspace at `{address}`")]
    #[diagnostic(
        code(shaipe::mcp::bridge),
        help(
            "`shaipe mcp --bridge` is started by an agent and connects back to \
             a workspace that is already running; it is not meant to be run by \
             hand. If you typed it yourself, you want `shaipe mcp <project>`. \
             If an agent did, its workspace has closed."
        )
    )]
    Bridge {
        /// Where it tried to connect.
        address: String,
        /// Why it could not.
        #[source]
        source: std::io::Error,
    },

    /// A bridge address could not be parsed.
    #[error("`{value}` is not a workspace address")]
    #[diagnostic(
        code(shaipe::mcp::invalid_address),
        help("Expected `unix:<path>` or `tcp:<host>:<port>`.")
    )]
    InvalidAddress {
        /// What was given.
        value: String,
    },

    /// The agent command is not on `PATH`.
    #[error("cannot find `{command}`")]
    #[diagnostic(
        code(shaipe::acp::agent_not_found),
        help(
            "Shaipe drives an agent you already have; it does not host one \
             (see ADR 004). Install OpenCode from https://opencode.ai, point \
             Shaipe at another ACP agent with `--agent \"<command> acp\"`, or \
             open the workspace without one using `--no-agent`."
        )
    )]
    AgentNotFound {
        /// What was looked for.
        command: String,
    },

    /// The agent process would not start.
    #[error("could not start `{command}`")]
    #[diagnostic(
        code(shaipe::acp::spawn),
        help(
            "The command was found but would not run. Try it yourself: it \
             should wait for JSON-RPC on standard input rather than printing \
             help and exiting."
        )
    )]
    AgentSpawn {
        /// What was run.
        command: String,
        /// Why it would not start.
        #[source]
        source: std::io::Error,
    },

    /// The agent has not finished with the last thing it was given.
    #[error("the agent is still working on the last prompt")]
    #[diagnostic(
        code(shaipe::acp::agent_busy),
        help(
            "Wait for the turn to finish, or press ctrl-c to stop it. The \
             workspace stays responsive either way — a prompt is never allowed \
             to queue behind another, because waiting for one would mean \
             waiting inside the event loop."
        )
    )]
    AgentBusy,

    /// The agent stopped, or was never running.
    #[error("`{command}` is no longer running")]
    #[diagnostic(
        code(shaipe::acp::agent_exited),
        help(
            "The agent exited during the session. Run it yourself with \
             `--print-logs --log-level debug` to see why; an unauthenticated \
             agent usually exits at once, and `opencode auth login` fixes that \
             one."
        )
    )]
    AgentExited {
        /// What was run.
        command: String,
    },

    /// The handshake did not complete.
    #[error("`{command}` did not complete the ACP handshake: {reason}")]
    #[diagnostic(
        code(shaipe::acp::initialize),
        help(
            "Shaipe speaks ACP protocol version 1, which is what every agent \
             shipping today negotiates. Check the agent is recent enough — \
             `opencode --version` should be 1.18 or newer."
        )
    )]
    AgentInitialize {
        /// What was run.
        command: String,
        /// What went wrong.
        reason: String,
    },

    /// The agent said something Shaipe could not act on.
    #[error("`{command}` sent something unexpected: {reason}")]
    #[diagnostic(
        code(shaipe::acp::protocol),
        help(
            "This is a bug in Shaipe or in the agent, not in the project. The \
             project is untouched. Rerun with `-vv` to log the exchange."
        )
    )]
    AgentProtocol {
        /// What was run.
        command: String,
        /// What went wrong.
        reason: String,
    },
}

impl Error {
    /// Attach a path to an [`std::io::Error`].
    ///
    /// Every I/O failure in Shaipe is about a file the user named or that the
    /// project referenced, and "No such file or directory" without the path is
    /// the least actionable message there is.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
