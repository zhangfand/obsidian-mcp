{
  description = "MCP server for Obsidian vaults";

  # A channel tarball rather than github:, so locking needs no GitHub API.
  # A consuming flake should set inputs.obsidian-mcp.inputs.nixpkgs.follows.
  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-unstable/nixexprs.tar.xz";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      manifest = builtins.fromTOML (builtins.readFile ./server/Cargo.toml);
      build = pkgs: pkgs.rustPlatform.buildRustPackage {
        pname = "obsidian-mcp";
        version = manifest.workspace.package.version or manifest.package.version or "0.0.0";
        src = ./server;
        cargoLock.lockFile = ./server/Cargo.lock;
        # CI runs cargo test; a test that needs more than the build sandbox
        # gives must not stop a deploy.
        doCheck = false;
        meta = {
          description = "MCP server for Obsidian vaults";
          mainProgram = "obsidian-mcp";
          platforms = systems;
        };
      };
    in
    {
      packages = forAll (pkgs: rec {
        obsidian-mcp = build pkgs;
        default = obsidian-mcp;
      });

      overlays.default = final: _prev: { obsidian-mcp = build final; };
    };
}
