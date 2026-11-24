# obsidian-mcp

An MCP server for Obsidian vaults, written in Rust. Agents find, read and
edit notes by name, alias, tag or property; links stay correct across
renames and moves; every reply fits in 8 KB.

```sh
obsidian-mcp --vault <dir> --listen 127.0.0.1:<port>   # MCP Streamable HTTP at /mcp
env/score --phase 1                                    # build, serve the test vault, score it
```

Tools and their arguments: `server/TOOLS.md`. Layout and owners: `AGENTS.md`.
How the test setup differs from production: `env/README.md`.

## Using it from a NixOS flake

```nix
inputs.obsidian-mcp.url = "git+ssh://git@github.com/zhangfand/obsidian-mcp";
inputs.obsidian-mcp.inputs.nixpkgs.follows = "nixpkgs";
# then: inputs.obsidian-mcp.packages.${system}.obsidian-mcp
# or:   nixpkgs.overlays = [ inputs.obsidian-mcp.overlays.default ];
```

The package builds for `x86_64-linux` and `aarch64-linux`.
