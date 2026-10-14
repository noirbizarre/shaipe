# ADR-031 — Navigate the workspace by workflow phase

## Status

Accepted; extends ADR-021 and ADR-030

## Context

The workspace originally put a description or transcript in the same box,
separate from a message editor, palette and references. Variant/render tabs
above one preview are the only artwork navigation. This makes a conversation
look like an alternative to durable intent, and leaves reference comparison,
review and repeated refinement without a place. Issue #33 settles the
interaction model before #34–#36 build the comparison, agent and history
widgets. ADR-021 already names workflow phases but explicitly does not track
their completion; ADR-030 makes the description and artwork durable and the
conversation transient.

## Decision

### Navigation and ownership

The primary navigation axis is **phase**, chosen explicitly as part of a
`from_scratch`, `reference` or `hybrid` workflow. A fresh workspace starts
with no workflow kind selected: the mere presence of a reference does not say
whether it is a source to reproduce or inspiration. `W` cycles kinds;
`,` and `.` browse their ordered phases, wrapping to permit another
construct/render/compare/refine loop. On changing kind a shared phase remains
selected; otherwise navigation begins at the first applicable phase. A
reference/hybrid kind can be selected before attaching a source reference to
plan work; evidence needing that source is unavailable until it exists.
The compact footer spells out kind and phase when it fits; under 60 columns it
uses `S`/`R`/`H` for the kinds and `in`/`an`/`st`/`co`/`re`/`cp`/`rf`/`va`
for the ordered phase names, leaving space for essential keyboard hints.

This is **location**, not progress. Selecting Validate does not approve an
iteration, finish the workflow or establish that Compare happened. Neither
ACP events nor tool calls advance the phase or claim a result was accepted.
`WorkflowKind::phases()` is the only phase list: from-scratch has no reference
analysis, strategy selection or comparison; its Inspect means inspect the
render. Variant, render specification and, later, iteration are independent
context within a phase rather than competing top-level navigation axes.

The SVG owns the description, drawing, variants, palette, references and render
specifications. The description is an ordinary editable project editor, not
the first chat message; user edits or `set_description` alter the project in
memory and are saved by the normal save action. A new session reads that
current description and those artifacts. Draft messages, transcript, tool
activity, phase selection, derived evidence, review choices and the chat's
layout/scroll position belong to the session, not SVG metadata. A transient
request such as “make the tail angular” does not itself update the description;
if angular geometry becomes durable intent, the agent reads the description
and updates it separately with `set_description`. Shaipe's own instructions
remain MCP instructions, not either kind of user text (ADR-022).

### Workspace surfaces

The target wide-screen arrangement for #34 and #35 is a compact phase/context
header over a dominant visual evidence surface, with a distinct description
editor and an independently openable agent workspace. The visual surface
chooses reference, current render, overlay or difference/heatmap, with named
comparison metrics alongside it when a source is available. Reference and
current render must be inspectable without losing the selected variant/render
context. From-scratch shows the render and its declared sizes; it does not
display disabled reference/difference placeholders. Use existing deterministic
render/analysis/compare services asynchronously; the UI does not calculate a
new similarity score. Visual evidence is recomputable and is not saved in SVG.

The phase, active variant/render context and dirty/on-disk indicators remain
identifiable while chat, a notice or a modal is open. Chat may be opened and
closed without swapping out the description, resized without resizing the
editor, and scrolled independently of the description and visual surface.
Opening chat does not implicitly change keyboard focus; explicitly focusing
its composer allows sending transient turns, and focusing its log allows
scrolling it. Mouse clicks focus the surface under the pointer and resize its
divider; the terminal's narrow layout gives the evidence the main area and
makes description and chat separately reachable destinations instead of
squeezing all three into unreadable columns. Neither chat nor an agent is
required to inspect, edit, render, save or navigate the project.

### Activity, review and intervention

The agent surface displays streamed messages and tool activity as session
events, not as project milestones. Show an explicit “not connected” state
without disabling the project workspace. Human intervention can edit the
description or artifacts directly, send a new transient instruction, redirect
the next turn or cancel a running one. Project mutations from Shaipe tools
are immediately visible as unsaved edits under the existing single-owner
message boundary; an ACP permission request for an agent's own tool is not
approval of an artwork iteration. Approve/reject applies to an
identified candidate iteration, never silently to all project changes or
the transcript. Rejection retains a way to inspect the candidate and return
to a chosen artifact state; approval records a review decision, not a save,
phase completion or comparison threshold crossing. #36 retains snapshots,
evidence, relevant activity and phase outside the source SVG. Save and disk
conflict resolution remain separate explicit actions.

The session-owned phase model, independent chat workspace and comparison
surfaces are implemented. Chat opens as a separately resizable side pane
without replacing the description or taking focus. On narrow terminals,
evidence, description and chat are distinct destinations (the latter two are
reachable by `d` and `t`, with `v` returning to evidence). The `c` key cycles
reference/current/overlay/difference evidence when a source is attached to a
reference or hybrid workflow; named metrics come from the existing comparison
service. Issue #36 adds bounded, in-memory history of complete SVG snapshots,
selected render context, phase, origin and asynchronously computed visual
evidence. One candidate is captured per agent turn with successful mutations,
completed direct edit or disk reload, rather than on each text keystroke;
identical document bytes do not create duplicates. Reference bytes are frozen
on capture so historical comparisons remain attributable even if a reference
file changes. At most twelve candidates and 64 MiB of decoded historical
images are retained; older unselected candidates are evicted at those limits,
with SVG state preferred over images when two protected candidates exceed the
image budget. A busy worker reports unavailable evidence instead of blocking
the workspace. Historical comparison reports an unavailable reference when
the input exceeds 16 MiB or 1600 pixels on either axis, avoiding unbounded
decoded evidence; the live comparison is unaffected. Closing the workspace
discards the history. The history strip uses `{`/`}` for earlier/later, `H`
for live, `A`/`J` for approve/reject and `U` to restore an
inspected snapshot explicitly as a new unsaved candidate. Reviewing never
mutates SVG; the live project must first reach a checkpoint to be reviewed,
and an active agent turn must finish or stop before restoring. Restoring
preserves the normal save and disk-conflict checks.

## Consequences

Phase navigation offers context without falsely certifying model behavior,
but a user must select a workflow kind before navigating phases. Repeated
work cannot be represented by a single monotonic completion bar. Keeping
evidence and iteration snapshots outside SVG avoids turning a normal SVG into
a session database. Small terminals need explicit destinations and focus rules
rather than proportional slices of every pane.
