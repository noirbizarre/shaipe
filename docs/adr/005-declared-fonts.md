# ADR-005 — Fonts are declared by the project, not resolved from the system

## Status

Accepted

## Context

`shaipe render` is meant to be a CI step: run it, and the committed assets are
either reproduced exactly or the build fails. Anything that makes a render
depend on the machine destroys that property, and fonts are the classic way it
happens — the same `<text font-family="Inter">` produces different glyphs, and
therefore different bytes, on a laptop that has Inter and a runner that does
not.

The natural fix is to embed the font in the document with `@font-face`. It does
not exist: `font-face` appears **nowhere** in the `resvg` source tree. Embedded
webfonts are silently ignored. The only font input is the `fontdb::Database`
handed to `usvg`, which makes font resolution Shaipe's problem rather than the
document's.

Loading the system's fonts up front would make text "just work", at the cost of
making every render machine-dependent and silently so.

## Decision

The project declares the fonts it needs:

```xml
<shaipe:font family="Inter" src="fonts/Inter.ttf"/>
```

`src` resolves relative to the project file, so committing the font next to
`logo.svg` is what makes CI draw the same glyphs.

System fonts are **never loaded speculatively**. Before rendering, the document
is asked which families it names — from `font-family` attributes and from
inline `style` declarations, excluding the CSS generics. Declared font files
are loaded. Only if a family is still unresolved does anything else happen:

- by default, the system fonts are loaded and a warning names the family, says
  the render may differ elsewhere, and says how to fix it;
- under `--strict-fonts`, it is an error.

## Consequences

- A project with no text — the common case for a logo — never touches the
  system font database at all, which is both faster and one less source of
  variance.
- `--strict-fonts` is a flag, not a project setting. It is a property of the
  invocation, not of the artwork; putting it in the project would make it
  something a project could switch off, which is backwards. The assets workflow
  passes it.
- Shaipe's own `logo.svg` contains no `<text>` at all: the wordmark is drawn as
  monoline geometry. That is the strongest form of the guarantee, and it is
  what the placeholder artwork does.
- Required families are read from the XML rather than from the resolved render
  tree, because the answer is needed *before* the database is built — and
  because what the author wrote is what the diagnostic should quote back.
- The system fallback remains the default because refusing to render a logo on
  a developer's laptop over a font they clearly have would be obnoxious. The
  warning, and the fact that CI is strict, is the compromise.
