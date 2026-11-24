# env

`env/score --phase <n>` builds the server, serves a fresh copy of the seeded
vault, and scores it with `verifier/run`. `env/test-score` proves `env/score`
against stand-ins. See `AGENTS.md` at the root for usage and exit codes.

## How this differs from production

| Here | Production (Mac mini) |
|---|---|
| x86_64 Linux, Rust 1.97 from rustup (CI pins 1.97.0) | aarch64 NixOS, built by `flake.nix` with nixpkgs' Rust (1.99.1 at the current `flake.lock`) |
| The server listens on loopback only, with no Cloudflare tunnel or Access in front | Reached through the Cloudflare tunnel behind Cloudflare Access |
| The seeded test vault `verifier/vault/`, copied fresh for each run | The real vault at `/var/lib/obsidian-zhangfan/vault` |
| Nothing else touches the vault while the verifier runs | Quartz rebuilds and the hourly git snapshot read and commit the same files |

Rule 11 (builds for the Mac mini, serves the zhangfan vault, the Monday
review completes) cannot be checked here. It is checked on the Mac mini
after a deploy. Here, `nix eval` of
`.#packages.aarch64-linux.obsidian-mcp.drvPath` only shows the aarch64
package evaluates; it does not build it.
