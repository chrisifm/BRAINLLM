# Publishing npm releases

A release requires public registry publication and anonymous installation verification. Building a local pack alone
does not publish packages. Check the registry for current versions; the checklist below applies to each release.

## Verified preparation (2026-10-28)

[Build-and-pack run 40730233593](https://github.com/chrisifm/BRAINLLM/actions/runs/40730233593) passed for all seven
native platforms and produced the npm tarballs from source `d5c3171`. Publication was skipped as intended for manual
dispatch. Since 2026-10-29 every `v*` tag publishes on its own: npm trusts `release.yml` in the `npm-publish` environment
(Trusted Publishing over OIDC, no stored token), and that environment only deploys from `v*` tags and `main` (manual dispatch with `publish=true`). Anonymous installation
verification is still required for every public release.
Later source changes require a new build-and-pack run before release.

## Release requirements

1. Review the intended source commit through the principal-author gate. An explicitly authorized release may tag
   the reviewed commit on its review branch; publication never authorizes an automatic merge into `main`. Run `bash scripts/verify.sh`,
   `node --test npm/test/pack.test.mjs`, and the source/history privacy audit. Review the frozen version.
2. Build and test the native binaries for all seven declared platforms. Refresh obsolete CI runner labels against
   the current GitHub runner catalog before dispatching. The binary must use remapped source/home paths.
3. Pushing the tag is the release decision: the `npm-publish` environment has no approval step, only the `v*` tag
   policy, so run every gate and the secret scan before tagging. Confirm the publish job includes `create-brainll` as well as
   `create-brainllm`, with platform packages first, then runtime/launcher, then both initializers.
4. Each of the 11 packages lists a trusted publisher on npmjs.com (package → Settings → Trusted Publisher: GitHub
   Actions, `chrisifm/BRAINLLM`, `release.yml`, environment `npm-publish`). A new package must be published once by
   the owner before it can get one. The publish job needs npm 11.5.1 or newer. Never put credentials in source, logs,
   issue comments or chat.
5. Pack the reviewed artifacts with `node npm/pack.mjs --bins /path/to/binaries --require-all`. Review each tarball's
   file manifest, license, version, checksums and absence of secrets. A JavaScript-only dry-run cannot prove the
   native release is installable.
6. The tag push publishes with `--access public` and the supported provenance
   mechanism in the release environment. Do not reuse an already published version or automatically retry an
   ambiguous upload; query the registry for that exact version first.
7. Verify anonymous clean installation on supported systems, including both commands below. Check generated app
   dependencies, `npm run dev`, `npx brain check --json`, `npx brain doctor` and `npm run build`. Only then change the
   README/landing from source-install status to npm availability.

```sh
npm create @christianirack/brainllm@latest canonical-demo
npm create @christianirack/brainll@latest shorthand-demo
```

The shorthand resolves to `@christianirack/create-brainll`; it is a separately published initializer with identical
behavior and the same pinned compiler dependency. Both packages need to exist for both commands to work.

No npm login or subscription is required to consume the public packages once published. A scoped name is a namespace,
not a private-access requirement. Public access is explicit in every generated package manifest.

References: [npm initializer resolution](https://docs.npmjs.com/cli/npm-init/),
[public scoped packages](https://docs.npmjs.com/creating-and-publishing-scoped-public-packages/).

Relative source and asset links in packaged README files are pinned to the package's v<version> tag. Package documentation therefore resolves to the same reviewed source tree even when main changes. Create the matching source tag with the release; publishing the package without that tag would leave these links unresolved.

## Post-publication CI recovery (2026-10-30)

Publication and consumer verification are separate jobs. A red registry-install job does not mean npm rejected the
packages. First inspect the publisher and all exact-version registry integrity/provenance records. Never rerun a
successful publisher, reuse a public version or move its source tag to repair a checker.

Use `verify-published.yml` on the reviewed CI repair branch to test an already public version without publishing,
packing, requesting OIDC or inheriting npm credentials. Both initializers must complete exact-version installation,
check, production build, doctor and an actual development HTTP response. Windows currently reports browser testing
as unsupported; do not describe that result as a successful CDP or accessibility execution.

The checker owns its home, npm configuration, cache, browser profile and temporary listeners. Linux CI explicitly
selects a disposable browser wrapper, matching the source-verification runner setup; this does not alter consumers'
browser security configuration. Failure artifacts contain only an allowlisted structured summary, never raw doctor
reports, application sources, user paths, environment values or caches.

The repaired publisher condition requires an uncanceled run and either an actual tag-push event or explicit manual
`publish=true`. A manual `publish=false` stays pack-only even when its selected ref is a tag. These corrections apply
to new reviewed source commits; the already published v0.4.0 tag remains immutable and retains its historical workflow.


## Source verification before future publication

Development 0.4.1 makes publication depend on `verify-source` as well as the package result. The source job calls
`verify.yml` from the same commit and runs the existing workspace/runtime/fixture/privacy/English gates; it inherits
no publishing credentials. Native builds may run concurrently, but a failed, skipped or canceled source gate cannot
publish even when a reviewed package archive is reused. Manual pack-only dispatch still does not publish.
Boolean-policy regressions exercise these failure and cancellation paths without contacting npm or publishing.
