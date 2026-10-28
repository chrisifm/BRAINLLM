# Browser support — INTERNALS §19 (verified 2026-10-13)

Base sources (downloaded 2026-10-13):

- **[BCD]** MDN browser-compat-data 8.1.5 (timestamp 2026-10-08): https://unpkg.com/@mdn/browser-compat-data/data.json
- **[WS]** webstatus.dev / web-features: `https://api.webstatus.dev/v1/features/<id>`
- **[CIU]** caniuse fulldata: https://raw.githubusercontent.com/Fyrd/caniuse/main/fulldata-json/data-2.0.json
- **[FF147]** https://developer.mozilla.org/en-US/docs/Mozilla/Firefox/Releases/147 · **[FF151]** https://developer.mozilla.org/en-US/docs/Mozilla/Firefox/Releases/151
- **[WK262]** https://webkit.org/blog/17640/webkit-features-for-safari-26-2/

| Feature | Chrome | Firefox | Safari | Baseline | Proposed text for the "Support" column of INTERNALS §19 | Source (URLs) |
|---|---|---|---|---|---|---|
| Navigation API (`window.navigation`, `NavigateEvent.intercept`) | 102 (`intercept` 105) | 147 | 26.2 | Newly (2026-01-13) | Baseline 2026: Chrome 102+, Firefox 147+, Safari 26.2+ | [WS] https://api.webstatus.dev/v1/features/navigation · [BCD] `api.Navigation`, `api.NavigateEvent.intercept` · [FF147] · [WK262] |
| View Transitions same-document (`document.startViewTransition`) | 111 | 144 | 18 | Newly (2025-10-14) | Baseline 2025: Chrome 111+, Firefox 144+, Safari 18+ | [WS] https://api.webstatus.dev/v1/features/view-transitions · [BCD] `api.Document.startViewTransition` · [CIU] `view-transitions` |
| View Transitions cross-document (`@view-transition { navigation: auto }`) | 126 | no (Level 1 only) | 18.2 | Limited | Chrome 126+, Safari 18.2+, no Firefox | [WS] https://api.webstatus.dev/v1/features/cross-document-view-transitions · [BCD] `css.at-rules.view-transition` · [CIU] `cross-document-view-transitions` (Firefox note #2: Level 1 only) |
| Popover API (`popover`) | 114 | 125 | 17 (iOS 18.3) | Newly (2025-01-27) | Baseline 2025: Chrome 114+, Firefox 125+, Safari 17+ | [WS] https://api.webstatus.dev/v1/features/popover · [BCD] `html.global_attributes.popover` |
| Invoker commands (`commandfor` / `command`) | 135 | 144 | 26.2 | Newly (2025-12-12) | Baseline 2025: Chrome 135+, Firefox 144+, Safari 26.2+ | [WS] https://api.webstatus.dev/v1/features/invoker-commands · [BCD] `html.elements.button.commandfor`, `api.CommandEvent` · [WK262] |
| CSS anchor positioning (`anchor-name` / `position-anchor`) | 125 (partial: no `position-visibility: anchor-valid`; `position-anchor: normal` since 151) | 147 (partial: does not animate `anchor()`; `normal` since 151) | 26 (partial); full in 27 | Limited | Chrome 125+, Firefox 147+, Safari 26+ (full only in Safari 27; partial elsewhere) | [WS] https://api.webstatus.dev/v1/features/anchor-positioning · [BCD] `css.properties.anchor-name`, `css.properties.position-anchor` · [CIU] `css-anchor-positioning` · [FF147] · [WK262] |
| `appearance: base-select` (customizable select) | 135 | behind a flag (149, `dom.select.customizable_select.enabled`) | 27 | Limited | Chrome 135+, Safari 27+, Firefox behind a flag | [WS] https://api.webstatus.dev/v1/features/customizable-select · [BCD] `css.properties.appearance.base-select` · [CIU] `customizable-select` |
| `@starting-style` | 117 | 129 | 17.5 | Newly (2024-08-06) | Baseline 2024: Chrome 117+, Firefox 129+, Safari 17.5+ | [WS] https://api.webstatus.dev/v1/features/starting-style · [BCD] `css.at-rules.starting-style` |
| Scroll-driven animations (`animation-timeline: scroll()/view()`) | 115 | no (Nightly only) | 26 | Limited | Chrome 115+, Safari 26+, no Firefox (Nightly only) | [WS] https://api.webstatus.dev/v1/features/scroll-driven-animations · [BCD] `css.properties.animation-timeline.scroll` / `.view` (Firefox `preview`) |
| Long Animation Frames (`PerformanceLongAnimationFrameTiming`) | 123 | no | no | Limited | Chrome 123+, no Firefox, no Safari | [WS] https://api.webstatus.dev/v1/features/long-animation-frames · [BCD] `api.PerformanceLongAnimationFrameTiming` |
| Event Timing (`PerformanceEventTiming`, the basis of INP) | 76 (`interactionId` 96) | 89 (`interactionId` 144) | 26.2 | Newly (2025-12-12) | Baseline 2025: Chrome 76+, Firefox 89+ (`interactionId` 144+), Safari 26.2+ | [WS] https://api.webstatus.dev/v1/features/event-timing · [BCD] `api.PerformanceEventTiming`, `.interactionId` · [WK262] |
| Document Picture-in-Picture | 116 (desktop; not Android) | 151 (desktop only) | no | Limited | Chrome 116+ and Firefox 151+ (desktop), no Safari | [WS] https://api.webstatus.dev/v1/features/document-picture-in-picture · [BCD] `api.DocumentPictureInPicture` · [FF151] |
| Speculation Rules (prerender/prefetch) | 109 (105 partial: same-origin prerender only) | no | 26.2 behind a flag (prefetch only) | Limited | Chrome 109+, Safari behind a flag, no Firefox | [WS] https://api.webstatus.dev/v1/features/speculation-rules · [BCD] `html.elements.script.type.speculationrules` |
| 103 Early Hints | 103 (HTTP/2+ only) | 120 | 17 (HTTP/2+ only) | No web-features entry | Chrome 103+, Firefox 120+, Safari 17+ (HTTP/2+) | [BCD] `http.status.103` · [WS] search returns nothing: https://api.webstatus.dev/v1/features?q=103 |
| `content-visibility: auto` | 85 | 125 | 18 (partial: find-in-page misses it); full in 26 | Newly (2025-09-15) | Baseline 2025: Chrome 85+, Firefox 125+, Safari 18+ (full in 26) | [WS] https://api.webstatus.dev/v1/features/content-visibility · [BCD] `css.properties.content-visibility.auto` · [CIU] `css-content-visibility` |
| `contain-intrinsic-size` | 83 | 107 | 17 | Widely (since 2026-03-18) | Widely available: Chrome 83+, Firefox 107+, Safari 17+ | [WS] https://api.webstatus.dev/v1/features/contain-intrinsic-size · [BCD] `css.properties.contain-intrinsic-size` |
| `<dialog>` | 37 | 98 | 15.4 | Widely (2024-09-14; newly 2022-03-14) | Widely available: Chrome 37+, Firefox 98+, Safari 15.4+ | [WS] https://api.webstatus.dev/v1/features/dialog |

## Differences from what INTERNALS §19 said when this was verified

- **Navigation API** — "recent Chromium, Firefox, Safari" is right but vague: it is now **Baseline 2026** (Firefox 147, Safari 26.2); cite versions.
- **View Transitions** — "partial in Firefox" is outdated for same-document: Firefox 144 ships it and it is **Baseline 2025**. Cross-document is still **Chrome 126+ and Safari 18.2+ only**; no Firefox. They need separate rows.
- **Speculation Rules + 103 Early Hints** — grouping both under "Chromium" is wrong: Speculation Rules is Chrome 109+ (Safari 26.2 only behind a flag), but **103 Early Hints works in all three** (Chrome 103+, Firefox 120+, Safari 17+, HTTP/2+ only).
- **Popover + invokers + dialog** — "Chromium for invokers" is outdated: invoker commands are **Baseline 2025** (Firefox 144, Safari 26.2). Popover is Baseline 2025 (January). `<dialog>` is widely available.
- **Anchor positioning** — "Chromium, Safari 26+" leaves out **Firefox 147+**. web-features marks it **Limited**: only Safari 27 meets the full set; Chrome and Firefox have gaps (`position-visibility`, animating `anchor()`, initial value `normal` of `position-anchor` until version 151).
- **base-select** — "Chromium" falls short: **Safari 27** ships it; Firefox only behind a flag.
- **content-visibility** — plain "Baseline" is imprecise: `content-visibility` is **Baseline 2025 (newly)**, not "widely"; Safari was partial from 18 to 25. `contain-intrinsic-size` is **widely** available.
- **@starting-style, scroll-driven** — they do not share a status: `@starting-style` is **Baseline 2024**; scroll-driven animations are **Limited** (Chrome 115+, Safari 26+, no Firefox). The "Chromium" label for scroll-driven leaves out Safari 26.
- **LoAF + Event Timing** — they do not share a status: LoAF is Chrome 123+ only, but **Event Timing is Baseline 2025** (Firefox 89+, Safari 26.2+). INP as a field metric (CrUX) is still Chrome's, though the underlying API is interoperable.
- **Document PiP** — "Chromium" leaves out **Firefox 151+ (desktop)**; no Safari.
