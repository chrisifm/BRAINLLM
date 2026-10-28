# Create a frontend your LLM can own

BRAINLLM gives your agent a language built for the job. This initializer scaffolds an app with the compiler version
pinned and installs its dependencies. Both `create-brainllm` and `create-brainll` package names use this implementation.

After the first verified public release:

```sh
npm create @christianirack/brainllm@latest my-app
cd my-app
npm run dev
```

Pass `-- --no-install` to skip dependency installation. The package build includes the root marketing README with
absolute image and documentation links. See the [repository](https://github.com/chrisifm/BRAINLLM) for current release
status and source installation. Packing these sources alone does not make the npm command available.
