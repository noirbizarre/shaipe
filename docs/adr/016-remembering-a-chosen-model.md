# ADR-016 — A chosen model is remembered per agent, in the OS config directory

## Status

Accepted

## Context

A model can be requested three ways: `--model`/`SHAIPE_MODEL` at startup, or
`M` in the workspace mid-session. All three are ephemeral — nothing is ever
written down, so a model picked yesterday has to be picked again today.
Combined with the agent's own default frequently not having vision (see
ADR-017), this means the workspace opens on the wrong model far more often
than it should, and the fix is retyping the same flag every single time.

The obvious place to *not* put this is the project file. ADR-001 already
rejected a `shaipe.toml` sidecar for a related-sounding reason — a second
place for state invites drift with the SVG's own `<metadata>` — but that
ADR is about the artwork's own data: the prompt, the palette, the fonts, the
specs. A chosen model is none of those. It says something about how *this
machine* drives *this agent*, not about the logo, and it should follow the
same agent across every project a person opens on this machine, not travel
with one project and reset on the next. Putting it in the project would
answer a question nobody asked ("which model made this artwork") by
misusing a field meant for a different one, and would make two projects
disagree about "the" model for the same agent for no reason connected to
either project.

`src/fonts/mod.rs` already established the shape for exactly this kind of
local, cross-run state: `directories::ProjectDirs`, at the OS's own
convention rather than a bespoke path, with a graceful fallback when the OS
cannot say where one is. That module uses `cache_dir()`, because a font
cache is disposable — deleting it costs one re-fetch. A remembered model
preference is not disposable in the same sense: nobody expects "clear my
cache" to also forget which model they use, so this is the crate's first
use of `config_dir()` instead.

## Decision

`src/settings.rs`, a new module: `Settings`, holding a `command line -> last
model id` map — not a single global preference, because a model id is an
agent's own vocabulary (see `AgentConfig::with_model`'s own doc comment): a
value that means something to `opencode` is not even guaranteed to parse
for a different agent, so a preference remembered for one must never be
handed to another.

```rust
pub struct Settings { /* command line -> model id, plus where it lives */ }

impl Settings {
    pub fn load() -> Self;                          // <config dir>/settings.json
    pub fn model_for(&self, command: &str) -> Option<&str>;
    pub fn set_model(&mut self, command: &str, model: &str); // saves, best-effort
}
```

Tolerant by construction, matching every other piece of optional local
state in this crate: a missing or corrupt file behaves exactly like an
empty one, and a failed write is swallowed rather than surfaced. Losing a
remembered preference must never be worse than never having had one, and
must never stop the workspace from opening.

Applying a saved preference at startup reuses the exact mechanism `--model`
already goes through — `src/cli/tui.rs` only has to fall back to
`Settings::load().model_for(&config.command_line())` when nothing was asked
explicitly, and `AgentConfig`/`find_model_choice` in `src/acp/agent.rs` do
the rest, unchanged. A saved id that no longer exists (a deleted model, a
switched provider) is reported exactly the way a mistyped `--model` already
is — never silently ignored.

Saving happens automatically, the moment the model actually in use changes,
for any reason: a saved preference reapplied, `--model`/`SHAIPE_MODEL`, or
`M`. This needed one real fix along the way: `src/acp/agent.rs` sent the
workspace its list of models once, right after `session/new`, *before*
attempting the startup `--model` switch — so if that switch succeeded,
nothing told the workspace, and the `current` flag stayed on whatever the
agent had opened with until the first *mid-session* switch corrected it.
Mid-session switches already resent the list via `model_choices_after_switch`
on success; the startup path now does the same. Without this, `App` had no
reliable way to know which model was actually current immediately after
opening, and neither this ADR's saving nor ADR-017's nudge could depend on
it.

`App::set_models` is where the save lives: whichever entry the agent marks
`current` is compared against what was last recorded, and only a genuine
change reaches `Settings::set_model` — the agent resends this list more
than once a session, and re-saving the model already saved would be a
write that changed nothing, touching a file's modification time for no
reason.

## Alternatives rejected

- **Store it in the project file.** Rejected above — this is not artwork
  metadata, and two projects driven by the same agent would otherwise be
  forced to disagree about it.
- **One global preference, regardless of agent.** Rejected: a model id is
  an agent's own vocabulary, and nothing stops someone from having more
  than one ACP agent installed.
- **An explicit "set as default" action, rather than automatic.** Rejected:
  the complaint this exists to fix is having to set the model *at all*,
  every time, and adding a second keypress just to make a choice stick
  would still leave that complaint half-solved.
- **`$XDG_STATE_HOME` specifically**, since a preference is closer to
  runtime state than to a user-edited config. Considered, and set aside:
  `directories::ProjectDirs::state_dir()` only exists on Linux, and every
  other platform would need the same `config_dir()` fallback this already
  uses — so there was nothing to gain by reaching for a narrower API only
  some machines have.

## Consequences

- A second small file, `<config dir>/settings.json`, alongside the font
  cache — human-readable, and one line to delete if a preference should be
  forgotten entirely.
- A preference follows the agent, not the project: opening a different
  project with the same agent command starts on the same remembered model,
  which is the point, not an accident.
- `src/acp/agent.rs`'s startup path now matches its mid-session one exactly
  in how it reports a switch — a small, independently useful fix, not a
  workaround kept local to this feature.
