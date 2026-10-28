# `.brain_env` fixtures (INTERNALS §36, B0720–B0724)

An app has exactly one `.brain_env`, so each case is a mini app that `run.mjs` generates in a temp dir
(`brain new .` + a `.brain_env` + one view). The B0723 value is assembled at run time: no secret-shaped string is
committed (GitHub push protection would block it).

| Code | Case | Where it must appear |
|---|---|---|
| B0724 | malformed line (`lower=1`) | `brain check` (needs `check_file` wired, #138) |
| B0723 | `PUBLIC_*` holding a secret-shaped value | `brain check` (#138) |
| B0720 | empty key in `[production]` | `brain build --env production` |
| B0721 | a view reads `PUBLIC_API_URL`, not declared | `brain check` (`file: ".brain_env"`, edit `PUBLIC_API_URL=`) |
| B0722 | a declared key nothing reads (info) | `brain doctor` |

Every case also checks the hard rule of §36: no value from `.brain_env` appears anywhere in the captured output
(`LEAK` fails the run). `MISS` means the code does not reach that command yet.

```bash
cargo build -p brain-cli && node bench/fixtures-pending/env/run.mjs
```

Move these into `examples/invalid` once every case prints `OK` (each needs its own app root, so they stay generated).
