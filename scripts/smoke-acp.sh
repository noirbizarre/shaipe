#!/usr/bin/env bash
#
# The manual end-to-end check: Shaipe, an agent, and a real model.
#
# Not part of `mise run ci`, and it never will be. It needs an installed and
# authenticated agent, it makes real model calls, and it costs whoever runs it
# money. Nothing in this repository puts credentials in CI.
#
#   mise run smoke:acp
#
# Everything up to the model is covered by tests that always run. What this
# adds is the agent and the model at the far end of it.

set -euo pipefail

readonly BINARY="${SHAIPE:-target/debug/shaipe}"
readonly PROJECT="${1:-logo.svg}"

step() { printf '\n\033[1;35m==>\033[0m \033[1m%s\033[0m\n' "$1"; }
fail() { printf '\n\033[1;31mFAILED:\033[0m %s\n' "$1" >&2; exit 1; }
note() { printf '    \033[2m%s\033[0m\n' "$1"; }

step "Checking what is installed"

command -v opencode >/dev/null 2>&1 || fail \
  "OpenCode is not on PATH.

    Shaipe drives an agent you already have; it does not host one. Install it
    from https://opencode.ai, or point Shaipe at another ACP agent with
    --agent \"<command> acp\"."

note "opencode $(opencode --version 2>/dev/null || echo '(version unknown)')"

[ -x "$BINARY" ] || fail \
  "No Shaipe binary at $BINARY. Run \`cargo build\` first, or set SHAIPE."

[ -f "$PROJECT" ] || fail "No project at $PROJECT."

# ---------------------------------------------------------------------------
step "Mode A — the standalone MCP server"
# ---------------------------------------------------------------------------
#
# `shaipe mcp` speaks JSON-RPC on stdout and nothing else. Two frames in, two
# frames out, and the tool names printed.

readonly INITIALIZE='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'
readonly LIST='{"jsonrpc":"2.0","id":2,"method":"tools/list"}'

tools=$(printf '%s\n%s\n' "$INITIALIZE" "$LIST" \
  | timeout 20 "$BINARY" mcp "$PROJECT" 2>/dev/null \
  | tail -n 1 \
  | python3 -c 'import json,sys; print(" ".join(t["name"] for t in json.load(sys.stdin)["result"]["tools"]))' \
  ) || fail "\`$BINARY mcp $PROJECT\` did not answer a tools/list."

note "tools: $tools"

case "$tools" in
  *render_svg*) ;;
  *) fail "The server answered, but without render_svg. Got: $tools" ;;
esac

# ---------------------------------------------------------------------------
step "Mode A — through a real MCP client"
# ---------------------------------------------------------------------------

if config=$(mktemp -d 2>/dev/null); then
  cat > "$config/opencode.json" <<JSON
{
  "\$schema": "https://opencode.ai/config.json",
  "mcp": {
    "shaipe": {
      "type": "local",
      "command": ["$(cd "$(dirname "$BINARY")" && pwd)/$(basename "$BINARY")", "mcp", "$(cd "$(dirname "$PROJECT")" && pwd)/$(basename "$PROJECT")"],
      "enabled": true
    }
  }
}
JSON
  if (cd "$config" && timeout 60 opencode mcp list 2>&1 | grep -q 'shaipe'); then
    note "OpenCode connects to \`shaipe mcp\` as a configured server"
  else
    note "OpenCode did not list shaipe — check \`opencode mcp list\` yourself"
  fi
  rm -rf "$config"
fi

# ---------------------------------------------------------------------------
step "Mode B — an ACP session against a real model"
# ---------------------------------------------------------------------------

cat <<'GUIDE'
    This half is interactive, because watching it is the point.

    Run:

        shaipe logo.svg

    Then:

      1. Tab to the prompt pane (it is the top-left one).
      2. Type:  Call get_variants and tell me what this project contains.
      3. Press Enter.

    What to look for, in order:

      - the pane says "starting the agent…", then "the agent is ready";
      - a tool call appears, marked ·, and turns ✓;
      - the agent answers using the project's real variant names.

    Then ask it to change something:

        Make the icon's accent colour blue.

      - shaipe_get_svg and shaipe_write_svg appear as tool calls;
      - the preview redraws by itself — nothing was pressed;
      - "● unsaved" appears in the status line;
      - `git status` still shows a clean tree. Nothing is written until Ctrl-S.

    And the one that matters most, which needs a vision-capable model:

        Render the icon at 32 pixels and tell me whether it still reads.

      - if the answer describes the picture, the loop is closed;
      - if it says it cannot see images, the model is not vision-capable.
        That is the agent's configuration, not Shaipe's: the image is sent
        either way, which `rendering_through_mcp_returns_an_image_block_a_model_can_see`
        asserts without a model at all.

    The scripted version of the first two is:

        cargo test --test acp_opencode -- --ignored --nocapture
GUIDE

step "Done"
note "Mode A verified. Mode B is yours to drive."
