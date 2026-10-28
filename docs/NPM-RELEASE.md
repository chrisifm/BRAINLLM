# First public npm release

The public GitHub repository and a local pack are not an npm release. This change prepares public packages; it does
not authenticate, upload tarballs or claim the initializers are already available.

## Verified preparation (2026-10-28)

[Build-and-pack run 40730233593](https://github.com/chrisifm/BRAINLLM/actions/runs/40730233593) passed for all seven
native platforms and produced the npm tarballs from source `d5c3171`. Publication was skipped as intended for manual
dispatch. The `npm-publish` environment now requires approval by the principal author. The public npm registry still
has no initializers for this project; registry authentication and anonymous installation verification remain outstanding.
Later source changes require a new build-and-pack run before release.

## Release requirements

1. Review and merge the intended source commit through the principal-author gate. Run `bash scripts/verify.sh`,
   `node --test npm/test/pack.test.mjs`, and the source/history privacy audit. Review the frozen version.
2. Build and test the native binaries for all seven declared platforms. Refresh obsolete CI runner labels against
   the current GitHub runner catalog before dispatching. The binary must use remapped source/home paths.
3. Confirm the `npm-publish` GitHub environment actually requires Chris's approval in repository settings. A YAML
   environment name alone does not enforce approval. Confirm the publish job includes `create-brainll` as well as
   `create-brainllm`, with platform packages first, then runtime/launcher, then both initializers.
4. Verify the publishing identity with `npm whoami --registry=https://registry.npmjs.org/` in the intended private
   environment. The account must own or have publish access to `@christianirack`. Complete npm's required login/2FA
   through its own interface; never put credentials in source, logs, issue comments or chat.
5. Pack the reviewed artifacts with `node npm/pack.mjs --bins /path/to/binaries --require-all`. Review each tarball's
   file manifest, license, version, checksums and absence of secrets. A JavaScript-only dry-run cannot prove the
   native release is installable.
6. After the maintainer authorizes this exact version, publish with `--access public` and the supported provenance
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
