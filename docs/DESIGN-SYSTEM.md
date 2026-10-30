# Project design foundation

`design.brain.json` is the versioned project foundation. Schema version 1 records the native token stylesheet, its SHA-256 revision, exact custom-property values and conditional declarations, token revisions, and project rules. It is project source, separate from generated assets.

The framework APIs are `design::baseline(source, token_file, previous)`, `preview_init(root, options)`, `query(root, target)`, `check(root)` and `gallery(root)`. The CLI and MCP adapters expose these operations without requiring an application build.

## Adopt an existing project

Initialization previews revision-checked change operations for `design.brain.json`. Apply those operations through the change planner; previewing does not write the baseline. An optional `expectedRevision` refuses a stale baseline. The existing `tokens.cssbrain` is authoritative, with `tokens.brain.css` supported for older projects. Adoption never rewrites that stylesheet, fonts, base paths or application data.

Unknown baseline fields and unknown token fields survive adoption. Older scalar representations are retained as legacy values. Removed token objects remain marked `presentInSource: false`; adoption does not silently erase project history. Newer unsupported schema versions are refused rather than downgraded. Invalid JSON is not overwritten.

`check` reports missing foundations, source drift, and added, removed or changed token names. These are nonblocking metadata warnings. A source-formatting edit can change the source revision without changing individual token revisions. Explicitly adopt the inspected source revision to update the foundation.

## Mobile-first rules

New foundations describe a mobile-first baseline with native controls and no CSS framework. Defaults include 44 CSS pixel touch targets, 16 CSS pixel input text, logical properties for RTL, safe-area insets, reduced-motion support, visible keyboard focus, and no automatic external font fetching. Existing explicit project rule values are preserved.

The state hierarchy is permission, loading, error, empty, then success; disabled behavior applies across states. A presentational component may have no asynchronous or permission state. These rules document the project contract; they do not prove accessibility, contrast, focus behavior, responsive layout, localization or runtime state transitions. Verify those behaviors in the rendered application.

## Private static gallery

`gallery` writes `.brain/design/gallery.html`, a searchable mobile-first reference with tokens, typography, spacing, borders, native controls, buttons, cards, a hero and component contracts. It is private generated metadata, outside application distribution. It does not execute application components, call resources, submit forms or fetch fonts. Its content security policy blocks network connections and external resources.

The gallery uses escaped metadata and safe native custom-property values. Root and layer declarations can inform the preview; conditional declarations remain documented. The preview is not a complete reproduction of CSS cascade, media/container conditions or actual application screens. Component entries show contracts and statically declared states rather than executing user code.

A revision mismatch while assembling token and component metadata aborts generation with a retry instruction. Generated output uses bounded writes and a private writer lock. Custom source tokens, including font families, remain authoritative; the gallery fallback is only for absent tokens.

## Mandatory agent blueprint and editable basics

`design::blueprint(root)` is the starting guide before each UI edit. It lists the current foundation, project-owned basic files, missing components, canonical seed props/emits/exposed methods, required token names, explicit import examples, edit locations and responsive/state/accessibility acceptance criteria. Canonical seed contracts are distinguished from existing customized source; inspect its manifest for the actual project contract.

`new_basics()` is a pure scaffold source constructor. It provides editable project-owned button, field, input, select, checkbox, dialog, container, card, hero, search and header components. Native kit source is reused for implemented controls; input/select import their project Field explicitly. New copies use 44 CSS pixel targets and 16 CSS pixel inputs where relevant, native controls, token references, responsive wrapping and no external framework/font fetching. Dialog uses the existing native urlState helper with an explicit urlKey for shareable modal state. The component source stays editable and extendible inside the project rather than an opaque remote UI package.

`preview_seed(root, options)` accepts optional names and expectedRevisions, creates only missing basics and preserves every existing file. Selecting input/select includes their Field dependency. Seeding does not edit tokens or adopt a baseline automatically. Existing projects should inspect the blueprint's required tokens before applying; missing theme definitions require a separate reviewed configuration. Explicit imports select project-owned basics where names also exist in the native kit.

`preview_configure(root, options)` accepts 1..32 token-name/value pairs plus optional expectedTokenRevision and expectedBaselineRevision. It prepares CAS operations for the token stylesheet and matching foundation, preserving unmodified source bytes and unknown baseline fields. Existing tokens must have one unconditional root declaration; conditional, scoped or repeated definitions require explicit source edits rather than flattening a theme. New names are appended in a root block. Values are bounded local CSS literals: arbitrary rules, URLs, escape sequences and remote font loading are refused. Configuration does not change fonts unless a font token is explicitly selected.

After applying either plan, refresh manifests/gallery and verify actual rendered mobile/tablet/desktop, RTL, focus, keyboard, safe-area, reduced-motion and state behavior. The checklist is mandatory guidance, not an assertion that static source proves those outcomes.
