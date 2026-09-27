# ADR-017 — Vision capability is read from models.dev, cached, never guessed

## Status

Accepted

## Context

Shaipe's entire premise is that an agent can *see* the render (ADR-004). A
model without vision cannot, and yet the `M` picker has never been able to
say which of an agent's models qualify — because ACP's own
`SessionConfigOption` carries no capability metadata at all. Confirmed by
reading OpenCode's own `buildModelSelectOptions()`: it sends exactly
`{value, name}` per choice, nothing more, to every ACP client including
this one. There is no protocol field to read this off of.

The tempting fallback — recognise vision-capable models by name, a pattern
list Shaipe maintains and updates by hand as providers ship new models — is
exactly the shape of thing ADR-004 exists to rule out. It would make Shaipe
an opinion-haver about models after all, just expressed as a `match` arm
instead of a provider integration, and it would be wrong the day after
whichever model it was last updated for ships a family it does not
recognise.

[models.dev](https://models.dev) is a maintained, public catalogue of
exactly this metadata — per-model modality and pricing data, kept current
by the same ecosystem shipping the models — and it is not a coincidence
that its own `provider/model-id` keys match ACP's `ModelChoice.id` exactly
(confirmed against a live fetch: `anthropic/claude-opus-4-5`,
`opencode/grok-code-fast-1`, and so on): OpenCode's own model selector is
built from this same catalogue. Asking it is asking the same source
OpenCode already trusts to build the list Shaipe is annotating, not
inventing a second, competing one.

Two fields in that catalogue look like the same signal and are not, which
was found by fetching a live copy and checking rather than assumed:
`attachment` and `modalities.input` (which either does or does not name
`"image"`) disagreed on roughly four percent of every model listed —
several providers mark `attachment: true` on models `modalities.input`
confirms are text-only. `modalities.input` is the one actually describing
what a model accepts, so it is the only one read here.

## Decision

`src/vision.rs`, a new module, mirroring `src/fonts/mod.rs`'s shape for a
second, unrelated network access:

```rust
pub struct Catalogue { /* "provider/model-id" -> has vision, plus where it's cached */ }

impl Catalogue {
    pub fn load() -> Self;                 // <config dir>/vision.json, disk only
    pub fn is_stale(&self) -> bool;        // missing, or older than a day
    pub fn has_vision(&self, id: &str) -> Option<bool>; // None: not known either way
    pub fn refresh(&mut self) -> Result<()>; // fetches, flattens, caches — blocking
}
```

The upstream catalogue is roughly five megabytes across two hundred
providers and eight thousand models. None of that shape is kept: `refresh`
reads only `modalities.input` per model and flattens the result down to the
one thing Shaipe's own picker needs, `"provider/model-id" -> bool` — the
same "translate once" firewall `src/acp/update.rs` already keeps between
ACP's own types and the rest of the workspace, applied here to a second
upstream schema Shaipe does not want spreading through the tree either.

Fetched at most once a day, and never on the path that opens the
workspace: whatever is cached — stale or not, even entirely missing — is
shown at once, and a stale cache is refreshed on a spawned blocking thread
whose result reaches the workspace as just another update the event loop
selects on, same shape as an agent's own. A missing or unreachable
catalogue answers `None` for everything, which the `M` picker treats as
"nothing bad to say about it" — an unclassified model is never hidden,
only one the catalogue actually confirms lacks vision is.

The `M` picker defaults to showing only confirmed vision-capable models,
with `ctrl-v` lifting that filter for the rare turn that genuinely does not
need one — editing the prompt's text or a palette's hex value asks nothing
of the agent's eyes. Confirmed vision-capable entries are marked with a
small glyph in the list, matching `ToolStatus::glyph`'s existing plain-text
style.

Nothing here switches a model on the user's behalf. When the model actually
in use is *confirmed* to lack vision — never merely unclassified — the `M`
picker opens on its own, once a session, already filtered. The choice
stays the user's: a model can cost differently from another, and Shaipe
guessing at that trade-off on someone's behalf is exactly the kind of
decision ADR-004 says is not Shaipe's to make, whether the guess is "which
model" or "switch away from this one."

## Alternatives rejected

- **A hand-maintained name pattern.** Rejected above — the actual reason
  this ADR exists, not a footnote to it.
- **A hard filter with no escape hatch.** Rejected: some turns are pure
  text, and refusing to show a model for one of those would be answering a
  question nobody asked.
- **Auto-switching to a vision-capable model when the current one lacks
  one.** Rejected: a silent switch has cost implications nobody asked
  Shaipe to decide on their behalf — see ADR-004 again. Opening the picker
  is a nudge; choosing is still a keypress away, deliberately.
- **Guessing from `attachment` instead of `modalities.input`.** Rejected
  once the two were checked against each other on live data and found to
  disagree often enough to matter — recorded here as a fact found by
  fetching, per `AGENTS.md`'s own instruction to verify rather than assume
  about an external API.

## Consequences

- This is the crate's second network-touching code path, after ADR-015's
  font fetch — named as such rather than left for someone to discover by
  grepping: a one-line forward pointer was added to ADR-015's own
  Consequences and to `src/render/mod.rs`'s doc comment, which had
  (incorrectly, as of this ADR) claimed the font fetch was the only
  network access anywhere in Shaipe. Neither claim was about rendering
  specifically being network-free — `AGENTS.md`'s invariant 1 already
  scopes that to what a render depends on, and this module is never on
  that path — but a stale uniqueness claim left standing next to a truthful
  narrower one is exactly the kind of thing worth fixing on sight.
- A one-line forward pointer was added to ADR-004 as well, the same
  courtesy ADR-011 already gave it: this is a *fetched and continuously
  refreshed* mapping, not an invented, hand-maintained list — the
  distinction that ADR-004 actually draws — but it is still an external
  fact about models now living in Shaipe's own cache, and that is worth
  naming plainly rather than leaving implicit.
- A machine with no network reachable ever sees an unfiltered `M` picker,
  every model unclassified — degraded, not broken.
- A model that gains vision support upstream is reflected here within a
  day, with no Shaipe release required — the opposite of what a
  hand-maintained pattern list would have needed.
