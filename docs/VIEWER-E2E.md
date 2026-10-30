# Viewer browser E2E lane

`brain test` prefers a connected Viewer browser for declared `.test.brain` scenarios. The same native `/@brain/test.html`, `window.__brainLoad` and `window.__brainStep` API runs in a newly opened tab. This is execution of explicit authored scenarios, not an autonomous model inventing steps or a fabricated browser transcript.

The optional integration detects Codex Viewer at `127.0.0.1:4315` (X-XV session headers) and Claude Viewer at `127.0.0.1:4313` (X-CV session headers), selecting a connected scripting-capable service with the expected identity. Probe output projects only service version, connected state, user-script capability and supported test operations. It does not inventory tabs, windows, accounts, cookies or profiles. With multiple connected browser profiles, set private `BRAIN_VIEWER_PROFILE` explicitly for the intended extension profile. The transport never silently claims another session window; it opens an isolated owned test window. Probe reports requiresProfile without exposing profile identities. No personal installation path is embedded in the framework. Node.js with fetch support is needed for this transport; a missing runner/service/extension yields an explicitly reported unavailable-Viewer headless fallback.

A connected extension with disabled user scripts, broken bridge API, timeout, failed scenario or failed owned-tab cleanup does not become a successful fallback. The CLI/MCP caller must inspect scenario results as well as transport `ok`: a successful transport can contain a failing test assertion. A failed connected Viewer lane fails the combined test run even if a separate headless lane would pass.

## Scope and cleanup

The transport accepts only explicit loopback HTTP origins with a port and app-relative `.test.brain` paths. It rejects credentials, remote origins, traversal, hidden paths, fragments/query strings and arbitrary caller-supplied JavaScript. It opens at most one test tab at a time and addresses only its newly returned numeric ID. Every run checks the exact project origin and `/@brain/test.html` path before acting. Finally closes that owned ID; it never uses `active`, URL fragments, tab inventories or broad group/window close operations.

The installed Viewer contract documents isolated `USER_SCRIPT` and page `MAIN` worlds. The native test API is a page global, so the integration uses `MAIN` exclusively in the guarded owned test page. It does not inject into unrelated pages, use account credentials or operate a site's session. Synthetic test interactions retain the existing native fixture behavior; they are not claimed to be trusted hardware input.

Limits: 32 files per Viewer suite, 256 declared scenarios per file, one bridge command in flight, up to 30 seconds per command, a total Viewer deadline capped at 180 seconds, 64 KiB stdin and 2 MiB response/output. DOM-label observations are bounded to 4096 elements/128 observations and explicitly truncated. A lost POST response is recovered only by its existing opaque call ID; an open action is never blindly retried. If its result cannot be recovered, cleanup is reported unknown rather than claiming no tab was created. All Node pipes and timers are bounded; the parent kills/waits its own Node process after deadline.

## Preserve the full AX and heap gates

Viewer performs real browser interactions and a qualified DOM-label heuristic. That heuristic is not `Accessibility.getFullAXTree`, computed accessibility, forced GC or heap profiling. Its observations remain in the Viewer provenance lane and do not replace the existing AX gate.

When Viewer is connected, the declared fixtures also replay in owned headless CDP for the full Chrome AX audit and optional `--heap` forced-GC inspector gate. Results identify both lanes. Headless replay failures, actual AX failures and leak findings still fail the combined run. DOM heuristic observations remain separate from full AX results to avoid treating approximate labels as computed accessibility. This separate replay costs extra time and can repeat fixture effects: author test fixtures against disposable/mocked project data and make test setup/cleanup explicit. It does not represent accessibility inspection of the user's exact Viewer DOM through CDP.

When Viewer is unavailable, the existing owned headless Chrome runner remains the explicit fallback, with the reason in each file report. No successful Viewer or full-AX coverage is inferred from health alone. Node transport contract tests use a mock service; actual connected-browser execution is a separate integration check performed against a disposable local fixture and must be reported as such.

The integration follows the installed Viewer plugin/runtime documentation; it does not copy account-specific plugin code or local machine paths into this public repository. It does not install a new extension, launch a separate LLM CLI or select an expensive model. Agents read the resulting structured test report and repair source through the normal lease/revision-checked changes protocol.

Hidden test tabs use the native bounded state flush rather than waiting indefinitely for a suspended animation frame. Visible tabs still prefer a rendering frame with a bounded timer fallback. The test adapter does not activate unrelated tabs to make a test finish.
