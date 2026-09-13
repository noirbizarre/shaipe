# ADR-015 — A font may be declared by a checksum-pinned URL, cached locally

## Status

Accepted

## Context

A project that declares text needs a font committed alongside it (ADR-005) —
otherwise a render silently depends on whatever the machine that produced it
happened to have installed. That is correct, and stays correct. But it means
every project using a common family like Inter carries its own copy of it,
and a user working on one asked, plainly: why does the font have to live in
*this* repository at all?

The obvious answer — fetch it live, every render — was considered and
rejected outright. `AGENTS.md`'s first invariant is exact about this:

> **Rendering is deterministic.** Same project bytes, same specification, same
> output bytes, on any machine. No clock, no network, no environment...

and `src/render/mod.rs`'s own doc comment made the same claim about the code,
not just the policy: "nothing in it reads the network, the clock or the
environment." A render that fetches a font live would produce different
bytes depending on whether the network was up at that moment, which is
exactly the failure mode invariant 1 exists to rule out — worse than a
system font, if anything, since at least a system font doesn't depend on a
socket succeeding.

But ADR-005's actual objection was never network access — the word does not
appear in it. Its objection is *silent machine-dependence*: `<text
font-family="Inter">` resolving to whatever font of that name a particular
machine happens to have, with no record of which one. A URL with a checksum
pinned in the project's own metadata is a different shape entirely. The
checksum, not the URL, is what the render actually depends on — the same
relationship a lockfile has to a package registry. Fetched once, verified,
and cached, a declared remote font is no less reproducible than a committed
file: the same bytes are used every time, on every machine, and a machine
with a warm cache never touches the network to get them.

## Decision

`<shaipe:font>` gains a second, alternative pair of attributes:

```xml
<shaipe:font family="Inter" href="https://.../Inter-Bold.ttf" sha256="<64 hex chars>"/>
```

Exactly one of `src` (a committed file) or `href` (a URL) is required.
`href` **requires** `sha256` — there is no unpinned-URL mode. Without a
pinned hash the cache key would have to be the URL itself, which is exactly
"trust whatever is there right now" wearing a different hat; the checksum is
what turns a fetch into a value with an identity of its own, independent of
whether the remote host is still up, still the same file, or still there at
all.

Resolving a font — local or remote — is `crate::fonts::resolve`, a new
module, not a change inside `src/render/`. This mirrors how ADR-014 confined
`vtracer`: a self-contained engine, called by whatever layer needs bytes,
never inlined into the module whose whole claim is that it does not do this
kind of thing. `render/fonts.rs`'s per-font loop now reads
`crate::fonts::resolve(project, font)?` instead of a direct `std::fs::read` —
same call site, same error shape, but the one function in the entire crate
that can make an outbound request lives somewhere whose job is exactly that,
and `render/`'s module doc now says so precisely: it resolves nothing itself,
it receives already-resolved bytes.

Caching is content-addressed: `<cache dir>/fonts/<sha256>`, at the OS's own
cache-directory convention (`directories::ProjectDirs`, which respects
`XDG_CACHE_HOME` on Linux and its per-platform equivalent elsewhere) — never
inside the project, so two projects pinning the same font's hash share one
download. A cache hit re-verifies the hash before trusting it (a corrupted
entry is a cache miss, not a checksum-mismatch failure); a cache miss fetches
via `ureq` (rustls, no extra feature flags needed for `https://`), hashes the
response with `sha2`, and either writes it to the cache on a match or refuses
to cache it — and refuses to return it — on a mismatch.

`--strict-fonts` treats a checksum-verified remote font exactly like a
committed local one: both are explicit, pinned declarations, the opposite of
the silent system fallback the flag exists to refuse. A cold cache with no
network available fails the same way a missing committed file always did —
loudly, not silently substituting something else.

## Alternatives rejected

**Fetch live, every render.** Rejected outright — directly contradicts
invariant 1 and the render module's own doc comment, discussed above.

**An unpinned URL, no checksum.** Considered and rejected: this is not
meaningfully different from system-font fallback, just with a different
single point of non-reproducibility (an HTTP response instead of a local
font database), and it gives `--strict-fonts` nothing to verify against.

**Fetch outside Shaipe entirely** — a project-side script populating a
gitignored `fonts/` directory, with Shaipe only ever reading `src=`. Viable,
and considered first; rejected because it does not do what was actually
asked (support fetching a font, not work around not having it), and it
pushes the pinning/caching discipline onto every project's own tooling
instead of giving it once, centrally, to every project that wants it.

**A convenience shorthand for known providers** (e.g. a `google-fonts:Inter`
scheme). Rejected for now: a literal URL and a literal checksum is the
smaller, provider-agnostic surface, and nothing here forecloses adding a
shorthand later if it turns out to matter.

## Consequences

- This is the first network-touching code path in the entire crate —
  confirmed by exhaustive search before writing this, not assumed. Named
  plainly here rather than left implicit.
- `AGENTS.md`'s invariant 1 and `README.md`'s two "no network" sentences are
  narrowed, not deleted: they now name this one, opt-in, checksum-pinned
  exception, the same way they'd need to if any future capability earned one.
- A cold cache means the *first* render of a newly-declared remote font on a
  given machine needs connectivity. A CI setup that wants a render step with
  no network access at all, ever, has two options: keep using `src=` (still
  fully supported, unchanged), or pre-warm the cache as its own step (e.g. an
  `actions/cache` entry keyed by the `sha256`) before calling `shaipe
  render`.
- New dependencies: `ureq` (the fetch), `sha2` (the checksum), `directories`
  (the cache location) — each justified in `Cargo.toml` the way every
  dependency here already is.
- ADR-005 is not superseded. Its conclusion — never load system fonts
  speculatively — is untouched; this adds a third source alongside "local
  file" and "system fallback," trusted by a pinned hash rather than by
  presence. A one-line forward pointer was added to ADR-005 instead, the same
  courtesy ADR-004 gave ADR-011.
