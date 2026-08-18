//! What Shaipe knows about starting OpenCode.
//!
//! The one agent-specific module in the crate, and it earns that by being the
//! only way Shaipe can keep an agent out of the project file.
//!
//! [ADR 012] concluded that an ACP client cannot restrict an agent's tools.
//! That is true of the *protocol* — nothing in `initialize` or `session/new`
//! restricts anything, and every capability field is a positive declaration of
//! what the **client** supports — and false of the *process*, which is the
//! level Shaipe operates at. Shaipe spawns the agent, so it chooses the
//! environment the agent starts in. See [ADR 013].
//!
//! Anything that is not OpenCode gets nothing from here, and is reported as
//! unrestricted rather than quietly assumed to be safe.
//!
//! [ADR 012]: https://github.com/noirbizarre/shaipe/blob/main/docs/adr/012-cannot-restrict-an-agents-own-tools.md
//! [ADR 013]: https://github.com/noirbizarre/shaipe/blob/main/docs/adr/013-restrict-the-agent-through-its-environment.md

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

/// The variable OpenCode reads a config fragment from.
///
/// Documented, and layer 6 of 8 in OpenCode's config resolution — above a
/// project's own `opencode.json`, below enterprise managed configuration,
/// which is the right thing to lose to.
///
/// `OPENCODE_PERMISSION` would be more surgical and is applied later still,
/// but it is undocumented: present in 1.18.18 with no public-API guarantee.
/// A restriction that quietly stops working is worse than one that never was.
pub const CONFIG: &str = "OPENCODE_CONFIG_CONTENT";

/// What Shaipe denies the agent.
///
/// `edit` covers edit, write and patch. `bash` closes the hole the first one
/// leaves: an agent that finds `edit` denied will reach for `echo > file`, and
/// a restriction with a shell-shaped gap in it is decoration.
///
/// Reading, globbing, grepping and fetching stay allowed. An agent that can
/// read `AGENTS.md` and the SVG it is editing does markedly better work, and
/// none of it can damage the project.
const DENIED: [&str; 2] = ["edit", "bash"];

/// Whether a command starts OpenCode.
///
/// The file name, so `/usr/bin/opencode` and a wrapper on `PATH` both count,
/// and `opencode-something-else` does not.
#[must_use]
pub fn is_opencode(program: &str) -> bool {
    Path::new(program)
        .file_stem()
        .is_some_and(|name| name.eq_ignore_ascii_case("opencode"))
}

/// The environment an OpenCode agent should be started with.
///
/// Empty for anything else. `inherited` is whatever the variable already held
/// in this process's environment, which is merged into rather than replaced —
/// overwriting it would silently discard a configuration somebody wrote.
///
/// Returns the variables to set, and a note for the user when something about
/// their own configuration could not be honoured.
#[must_use]
pub fn restrictions(
    program: &str,
    inherited: Option<&str>,
) -> (BTreeMap<String, String>, Option<String>) {
    if !is_opencode(program) {
        return (BTreeMap::new(), None);
    }

    let (mut config, note) = match inherited {
        None => (json!({}), None),
        Some(existing) => match serde_json::from_str::<Value>(existing) {
            Ok(Value::Object(map)) => (Value::Object(map), None),
            // Left alone rather than replaced. Reporting it beats discarding
            // it, and beats refusing to start over somebody's typo.
            _ => (
                json!({}),
                Some(format!(
                    "{CONFIG} is set but is not a JSON object, so it was replaced \
                     rather than added to"
                )),
            ),
        },
    };

    // Set inside whatever was there, so every other key — the model, the
    // provider, the MCP servers, the plugins — survives untouched. OpenCode
    // deep-merges this fragment over the user's own configuration.
    let permission = config
        .as_object_mut()
        .expect("an object was just constructed")
        .entry("permission")
        .or_insert_with(|| json!({}));

    if !permission.is_object() {
        *permission = json!({});
    }

    for tool in DENIED {
        // A scalar, deliberately: it replaces the user's rule map for that key
        // rather than adding a rule that their own patterns could out-match.
        // A blanket deny is the point.
        permission[tool] = json!("deny");
    }

    let mut env = BTreeMap::new();
    env.insert(
        CONFIG.to_owned(),
        serde_json::to_string(&config).expect("a JSON object serialises"),
    );

    (env, note)
}

/// What to tell the user about an agent Shaipe could not restrict.
#[must_use]
pub fn unrestricted_note(program: &str) -> Option<String> {
    (!is_opencode(program)).then(|| {
        format!(
            "`{program}` is not an agent Shaipe knows how to restrict, so it can \
             edit files directly. Its own configuration is the place to stop that."
        )
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// The config fragment `restrictions` would set, parsed back.
    fn fragment(inherited: Option<&str>) -> Value {
        let (env, _) = restrictions("opencode", inherited);
        serde_json::from_str(&env[CONFIG]).expect("what we set is JSON")
    }

    #[test]
    fn the_recipe_denies_edits_and_the_shell() {
        // `edit` covers edit, write and patch; `bash` closes the hole it
        // leaves, because `echo > file` is a write by another name.
        let config = fragment(None);
        assert_eq!(config["permission"]["edit"], "deny");
        assert_eq!(config["permission"]["bash"], "deny");
    }

    #[test]
    fn reading_is_left_alone() {
        // An agent that can read the project does better work, and reading
        // cannot damage it.
        let config = fragment(None);
        assert_eq!(config["permission"].get("read"), None);
        assert_eq!(config["permission"].get("grep"), None);
    }

    #[test]
    fn an_existing_config_is_added_to_rather_than_replaced() {
        // The variable may already be set. Overwriting it would silently
        // discard whatever it carried — a model, a provider, an MCP server.
        let config = fragment(Some(
            r#"{"model":"anthropic/claude","mcp":{"other":{"type":"local"}}}"#,
        ));

        assert_eq!(config["model"], "anthropic/claude");
        assert_eq!(config["mcp"]["other"]["type"], "local");
        assert_eq!(config["permission"]["edit"], "deny");
    }

    #[test]
    fn existing_permissions_survive_except_the_two_that_are_denied() {
        let config = fragment(Some(r#"{"permission":{"read":"allow","edit":"allow"}}"#));

        assert_eq!(config["permission"]["read"], "allow");
        assert_eq!(
            config["permission"]["edit"], "deny",
            "the user's own allow should not outrank the restriction"
        );
    }

    #[test]
    fn a_config_that_does_not_parse_is_reported_rather_than_ignored() {
        let (env, note) = restrictions("opencode", Some("{not json"));

        assert!(note.is_some_and(|note| note.contains(CONFIG)));
        // And the restriction still applies: a typo in somebody's environment
        // must not be a way to switch it off.
        let config: Value = serde_json::from_str(&env[CONFIG]).unwrap();
        assert_eq!(config["permission"]["edit"], "deny");
    }

    #[test]
    fn a_permission_key_that_is_not_an_object_is_replaced() {
        // `{"permission": "allow"}` is valid OpenCode config. Indexing into a
        // string would panic, and honouring it would switch the restriction
        // off.
        let config = fragment(Some(r#"{"permission":"allow"}"#));
        assert_eq!(config["permission"]["edit"], "deny");
    }

    #[test]
    fn an_agent_that_is_not_opencode_gets_no_environment() {
        // Shaipe knows one agent family. Guessing at another's configuration
        // would be worse than leaving it alone and saying so.
        let (env, _) = restrictions("some-other-agent", None);
        assert!(env.is_empty());
        assert!(unrestricted_note("some-other-agent").is_some());
        assert!(unrestricted_note("opencode").is_none());
    }

    #[test]
    fn opencode_is_recognised_however_it_is_spelled_on_the_command_line() {
        for program in [
            "opencode",
            "/usr/bin/opencode",
            "/home/me/.local/bin/opencode",
        ] {
            assert!(is_opencode(program), "{program}");
        }
        for program in ["opencode-next", "claude", "/usr/bin/codex"] {
            assert!(!is_opencode(program), "{program}");
        }
    }
}
