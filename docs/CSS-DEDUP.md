# Encapsulated CSS and exact rule interning

A class's styling identity is its component scope plus its name. BRAINLLM already emits `@scope ([data-b="component"]) to ([data-b])` inside a cascade layer; the lower boundary prevents rules from entering a child component. Slot markup retains its author's ownership. Source class names stay readable, and production class renaming keeps the same map in selectors, compiler-owned markup and conditional class operations.

This change does not replace encapsulation with global utility classes. Different component scopes retain their boundaries. A restricted adjacent private-scope union, described below and checked in real Chrome, can share an exactly identical body without creating global utility classes. General root union without a DOM/selector ownership proof could change scope proximity, matching sets and the cascade.

## Exact interning

`brain_css::intern::dedupe` and `brain_pack::cssdedup::dedupe` return optimized generated CSS plus metadata. They reuse the existing in-house CSS parser, preserve the original text of declaration values and operate only on adjacent siblings within a proven equivalent context.

Two adjacent rules with identical ordered declaration bodies can share one plain selector list:

```css
@scope ([data-b="one"]) to ([data-b]) {
  .primary { color: var(--accent); }
  .highlight { color: var(--accent); }
}
```

becomes one body under the same scope:

```css
@scope ([data-b="one"]) to ([data-b]) {
  .primary,.highlight { color: var(--accent); }
}
```

Each selector keeps its own specificity. The optimizer never rewrites that list as `:is(.primary,.highlight)`. Initial support includes simple class, ID and type selectors, ordinary combinators, and the compiler's existing root anchors. Distinct complex/unknown pseudo selectors are left separate: an unsupported member of a selector list can invalidate the entire list in a browser. An identical adjacent rule may still be removed without introducing a new selector member.

Adjacent wrappers may be combined only when their headers are identical and their enclosing context is the same: `@media`, `@supports`, explicit `@scope`, or a named `@layer`. This preserves source order while exposing adjacent identical bodies inside those wrappers. Anonymous layers remain distinct. Different layer names, conditions, lower boundaries and parent contexts never share an interned body. Different scope roots are eligible only for the restricted private-component rule below.

Qualified-rule nesting, keyframes, font registrations, unknown groups and header comments are conservative boundaries. A comment between rules also prevents selector-list merging. Declaration order, fallback order, `!important`, custom-property tokens and string whitespace are preserved. A one-character value difference, a different fallback order or different custom-property token text keeps the rules separate. No shorthand algebra, property sorting, fuzzy similarity or global nonadjacent duplicate deletion is attempted.

## Adjacent private component scopes

The pass also admits adjacent private scopes with literal, mutually exclusive `data-b` roots, the exact lower boundary `to ([data-b])`, the same enclosing context, and byte-identical simple rule bodies. It joins the root selector list and emits that body once:

```css
@scope ([data-b="a"],[data-b="b"]) to ([data-b]) {
  .item { color: var(--accent); }
}
```

The [CSS Cascade 6 Editor's Draft, §3.5](https://drafts.csswg.org/css-cascade-6/) defines one scope per matched root, descendant limits, and scoped declarations whose specificity does not include the root selector. From those rules, this restricted union preserves each subject's root and proximity: literal `data-b` values are mutually exclusive, and another `data-b` descendant closes the outer scope. That is a proof for this structure, not general scope equivalence. Nesting, `&`, unfamiliar selectors/limits, nonadjacent blocks and any body difference remain excluded.

A real Chrome 156.0.8078.76 run compared original and optimized computed styles for eight fixtures: sibling and nested component roots, an intervening override, minimal body differences, layer/media conditions, different limits, inherited custom properties, and public class collisions. Positive assertions ensure styles actually apply and foreign roots remain excluded. The browser run uses the optimizer's real output followed by the production minifier; it does not duplicate the algorithm in JavaScript.

## Why the previous global scope deletion was unsafe

The old implementation remembered an entire `@scope` block without its containing media/supports/layer context. An identical block in a different condition was then deleted. It could also delete the final red rule in this sequence:

```css
@scope (.root) { .item { color: red } }
@scope (.root) { .item { color: blue } }
@scope (.root) { .item { color: red } }
```

The final override is observable. The new pass can combine those identical scope headers, but it preserves all three ordered rule bodies. It does not infer equivalence across an intervening rule.

## Production name collision protection

A short production name must not collide with a class which is outside the rename map. The name allocator reserves:

- Every component stylesheet class, including preserved candidates.
- Classes from public stylesheets, even when no component stylesheet declares them.
- Explicit foreign string words, including DOM selector and `classList` contracts.
- Literal markup classes which have no matching component stylesheet declaration.

For example, public `.a` or markup `class="a"` prevents `long-component-name` from becoming `a`. Explicit foreign references to a source class also keep that source name unchanged. Opaque encoded JS/CSS/HTML names and lexer failures disable unsafe renaming instead of guessing. The allocator is deterministic across input iteration order and does not assign the same short name to two distinct candidates.

Computed external class names remain an integration boundary: declare externally styled classes in public CSS and use explicit adapter contracts. This pass cannot prove every runtime-generated name of an opaque SDK. It never rewrites external stylesheets or arbitrary SDK strings.

## Feedback and limits

`Report::to_json()` contains `schemaVersion`, `coordinateSpace: "generated-css"`, input/output byte counts, merged-rule/group counts, `scopeBodiesInterned`, duplicate-rule counts, a skip reason and bounded changes with line/column/kind. These coordinates refer to the generated CSS passed into this transform, not to an invented `.brain` source location. The caller may attach a generated filename. CSS values, selectors and absolute local paths are absent from feedback.

Input is capped at 4 MiB and 64 brace levels; unsupported/malformed input is returned unchanged. Change lists retain at most 256 entries with `changesTruncated`; aggregate counts still describe the whole transform. These are raw transform bytes. Compressed savings must be measured from final emitted assets using the output ledger, not inferred from source length.

## Remaining global reuse work

Existing module/chunk hashing already reuses byte-identical emitted modules. This increment shares adjacent rule bodies, compatible wrappers and the proven adjacent private-component scope bodies above. It does not claim full global CSS interning across arbitrary components or nonadjacent cascade positions. That next step requires a proof of selector/DOM ownership, specificity, scope proximity and intervening declaration interactions before a shared internal class can replace distinct source classes. A merely similar declaration set is never sufficient.

Regression coverage includes isolated component scopes, minimal declaration differences, media/supports/layer boundaries, nested contexts, specificity, override order, custom-property whitespace, fallback order, generated-coordinate privacy, public/foreign name collisions and deterministic allocation.

The production minifier now protects custom-property values before whitespace or value shortening and restores their original token text in one pass. Parsing/printing also preserves those raw values. A custom `--gap:0px` must retain its length unit when substituted into `calc(var(--gap) + 1px)`; a unitless number would change validity. Empty values, trailing whitespace, comments and fallback order remain intact. Opaque custom blocks fail closed in the production minifier. Ordinary shortening is restricted to declaration values and skips URL tokens, so selector IDs and SVG fragments such as `#aabbcc` do not become color shorthands.
