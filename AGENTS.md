# obsidian-mcp

An MCP server, written in Rust, that lets agents work with an Obsidian vault
in Obsidian's terms: notes, links, sections, properties, tags. The owner
judges it only by the pass or fail rules the verifier scores.

## Folders and owners

| Folder | Owner | Holds |
|---|---|---|
| `server/` | product builder | the Rust server (`obsidian-mcp --vault <dir> --listen 127.0.0.1:<port>`, MCP at `/mcp`); tool shapes in `server/TOOLS.md` |
| `verifier/` | verifier builder | the seeded test vault (`verifier/vault/`), the expected answers, the scoring program `verifier/run`, and `verifier/selftest` |
| `env/` | environment builder | `env/score`, its own test `env/test-score`, and the notes in `env/README.md` |

The environment builder also owns `.github/`, `flake.nix`, `flake.lock`,
`.gitignore`, `README.md` and this file. A builder changes nothing outside
what it owns. Nobody merges or pushes to `main` but the planner.

## Scoring

```sh
env/score --phase 1
```

It refuses to run when anything in the repository is uncommitted, because a
score describes a commit. It builds `server/` in release mode, serves a fresh
copy of `verifier/vault/` on a free loopback port, runs `verifier/run`, and
prints the verifier's report, the commit and `rustc --version`. Its exit code
is the verifier's: 0 only when every rule of the phases up to `--phase`
passes. Exit 2 means bad usage or uncommitted changes; 3 means the build or
the server start failed. The server log and `report.json` are left in
`env/out/` (ignored by git).

`env/test-score` checks `env/score` itself against stand-ins in a temporary
repository.

## Other checks

```sh
(cd server && cargo test)
verifier/selftest
nix build .#obsidian-mcp            # the package the Mac mini's flake takes
```

CI (`.github/workflows/ci.yml`) runs all of these except `nix build` on each
push.

## Writing

Code comments say what the code cannot. No CLAUDE.md files. Commit messages
have an imperative subject and a body that says why.
