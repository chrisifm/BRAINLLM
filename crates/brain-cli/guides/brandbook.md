# guide: brandbook — a living public reference for every site

## When
Start here before choosing a site's identity, writing its voice or changing its design. Every new static and ordinary
project includes /brandbook. Update it alongside brand, copy and UI changes rather than writing a separate final report.

## Authority and update loop
1. Read the existing /brandbook, `brain design --json`, `brain manifest --json` and the current source tokens.
2. Record public editorial decisions in `design.brain.json.brandbook.sections`. Use the stable IDs below. Preserve
   unrelated design metadata and approved decisions. Do not put analytics IDs, secrets, private research or working files here.
3. Each entry has `status`, `text` and optional `evidence` (a list of public source descriptions). Status is draft,
   proposed, approved, implemented or not-applicable. Default writing prompts stay in the editable contract;
   untouched drafts show a brief pending-review notice. Customized text, statuses and evidence are rendered in full.
   In multilingual projects, `brain add locale` / `brain fix` register generated messages and preserve their keys in
   `brandbook.messages`. Translate the normal locale files; keep that identity map. Check/build only read it, and
   the owned view placeholder stays empty. Later editorial changes receive the usual B1403 extraction edits.
4. Use draft/proposed for assumptions. Approved needs the project owner's decision; implemented needs actual source
   evidence. The compiler never promotes a status or invents approval. Explain why a section is not applicable.
5. Keep the single empty owned section in views/brandbook.brain on its own line. Check, dev and build expand it from
   the same current inputs. The generated token table reads the actual stylesheet; the UI inventory reads existing
   files. These observations do not prove accessible behavior, brand approval or computed contrast.
6. Run `brain check`, `brain test`, `brain doctor` and `brain build`. The next build refreshes derived facts without
   rewriting editorial JSON or the authored view. Keep the brandbook route linked in the site navigation.
7. Review content whenever the audience, offer, voice, assets, components, accessibility or publishing policy changes.
   Keep the default noindex while it contains unapproved drafts; remove it deliberately when the reference is ready.

## How the LLM writes
Use the project's approved language and voice. Prefer short, concrete statements, observable rules and useful examples.
Say what to do, in which situation and why. For voice/microcopy, include good/bad examples and recovery wording.
Distinguish a rule, an example, a hypothesis and a measured finding. Give public sources, dates and limits for claims.
Never fabricate an audience, testimonial, metric, approval, legal conclusion or capability. Do not copy another brand's
phrases, assets, palette or identifiers. Research is a reference, not permission to impersonate the reference brand.
Document color/type roles in editorial text; current values come from tokens. Change the real design source first,
then review the regenerated inventory. Missing decisions stay visible drafts rather than polished invented content.
Text is escaped, not executable markup. Evidence is displayed as plain public source descriptions. Use authored
components outside the owned section when a real visual example is needed; the generator does not execute components.

## Required structure and stable IDs
All groups remain visible; use not-applicable with a reason instead of silently dropping an irrelevant section.

| Group | IDs | What belongs here |
|---|---|---|
| Strategy | purpose, audience, evidence, personas, segments, personality, locales | Purpose/promise/values, audience needs, research limitations, illustrative personas, segments, mood and language decisions. |
| Voice | voice, writing, verbal-platform, signature-lines, connection-lines, topics, microcopy | Situational tone, terminology, positioning, approved phrases, content scope and interface copy. |
| Identity and product | logo, clear-space, misuse, color, typography, surfaces, motion, support-assets | Approved assets, size/space constraints, misuse, semantic color/type roles, surfaces and reduced motion. |
| Identity and product | ambassador, illustration, iconography, components, accessibility | Optional representative, visual grammar, accessible icons, component/state contracts and accessibility requirements. |
| Identity and product | seo, social-preview, email, implementation | Public metadata/indexing, sharing previews, email standards and public implementation guidance. |
| Assistant | assistant-scope, assistant-style, assistant-boundaries | Tasks actually supported, response style, uncertainty, sensitive-data limits and handoff. |
| Channels and governance | social, video, advertising, requirements, ai-policy | Context-specific formats, truthful claims, mandatory elements and AI assistance/review rules. |
| Channels and governance | brand-policy, legal, governance, downloads | Consistency/exceptions, reviewed public policies, ownership/approval/history and approved downloads. |
| Research | competition, benchmarks, gaps, product-standard, risks, crisis | Cited comparisons, reference criteria, open decisions, observable product quality and escalation. |
| Research | experiments, measurement, inclusion, emerging-tech | Hypotheses/tests/findings, outcome measurement, respectful language and relevant technology evidence. |
| Research | technology-limits, audience-context, technology-language, references | Limits, audience situations, accurate terminology, public sources and review dates. |

## Minimal correct example
Keep this field inside the existing design.brain.json object; do not replace its tokens, rules or other metadata:
```json
{
  "brandbook": {
    "schemaVersion": 1,
    "sections": {
      "voice": {
        "status": "proposed",
        "text": "Use direct, specific language. Errors name what failed and give one useful recovery action. Review this proposal with the project owner.",
        "evidence": []
      }
    }
  }
}
```
The rest of the required sections remain visible drafts. `text` contains 1..4000 bytes; each section has at most eight
source descriptions of 1..512 bytes. The design file stays at most 1 MiB and the generated section at most 512 KiB.
The source placeholder is structural: a string inside a script is not a marker, and duplicate markers fail B2203.

```html
<section data-brain-brandbook="content"></section>
```

## Verify
An older project gets B2202 as a warning when its design foundation has no brandbook route. Add it with
`brain add view brandbook`; read its generated test and keep the existing application branding. An already authored
brandbook without the marker is preserved; its author maintains it manually. No generated view overwrites that page.

```sh
brain guide brandbook
brain check --json
brain build --static --site https://example.com --json
```
Use the real public origin. The static flag is for static projects; ordinary projects use their normal build.
Inspect /brandbook on the output host and verify its token values after changing a source token. Confirm editorial
JSON remains unchanged, approved entries remain approved and new/deleted UI files update the inventory.

## Common mistakes
| Mistake | Code | Fix |
|---|---|---|
| Older design-enabled project has no brandbook | B2202 | Add the view and review its editorial content. |
| Invalid section, status or duplicate placeholder | B2203 | Use the stable IDs and one canonical empty owned section. |
| Treating source observations as approval or measured accessibility | — | Keep claims in draft until actual evidence supports them. |
