{
  description = "ai: all-in-one AI tool";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      forAllSystems = lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          ai = pkgs.rustPlatform.buildRustPackage {
            pname = "ai";
            version = (lib.importTOML ./Cargo.toml).package.version;
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./tests
                ./ai/src/rs
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            nativeCheckInputs = [ pkgs.git ];
            postCheck = ''
              for crate in memory chat; do
                cp Cargo.lock ai/src/rs/$crate/
                CARGO_TARGET_DIR=$PWD/target cargo test --release --offline \
                  --target ${pkgs.stdenv.hostPlatform.rust.rustcTarget} \
                  --manifest-path ai/src/rs/$crate/Cargo.toml
              done
            '';
            meta.mainProgram = "ai";
          };
        in
        {
          inherit ai;
          default = ai;
        }
      );
    };
}
