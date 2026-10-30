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

1. Review and merge the intended source commit through the principal-author gate. Run `bash scripts/verify.sh`,
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
