{
  description = "subraumctl - MQTT CLI/TUI for the subraum";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    flake-parts.url = "github:hercules-ci/flake-parts";
    crane.url = "github:ipetkov/crane";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = inputs @ { flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "x86_64-linux" "aarch64-linux" ];

      imports = [ inputs.treefmt-nix.flakeModule ];

      perSystem = { system, ... }:
        let
          pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [ inputs.rust-overlay.overlays.default ];
          };

          rustTarget = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          craneLib = (inputs.crane.mkLib pkgs).overrideToolchain rustTarget;
          src = craneLib.cleanCargoSource ./.;

          commonArgs = {
            inherit src;
            strictDeps = true;
            SSL_CERT_FILE = "${pkgs.cacert}/etc/ssl/certs/ca-bundle.crt";
            nativeBuildInputs = with pkgs; [
              cmake
              pkg-config
            ];
            buildInputs = with pkgs; [
              openssl
              paho-mqtt-c
            ];
          };

          cargoArtifacts = craneLib.buildDepsOnly commonArgs;

          mcpArgs = commonArgs // {
            cargoExtraArgs = "--features mcp";
          };
          mcpArtifacts = craneLib.buildDepsOnly mcpArgs;

          mkPackage = extraArgs: craneLib.buildPackage (commonArgs // {
            inherit cargoArtifacts;

            postFixup = ''
              mkdir -p $out/share/man/man1
              $out/bin/subraumctl man --output-dir $out/share/man/man1

              mkdir -p $out/share/bash-completion/completions
              $out/bin/subraumctl completions bash \
                > $out/share/bash-completion/completions/subraumctl

              mkdir -p $out/share/zsh/site-functions
              $out/bin/subraumctl completions zsh \
                > $out/share/zsh/site-functions/_subraumctl

              mkdir -p $out/share/fish/vendor_completions.d
              $out/bin/subraumctl completions fish \
                > $out/share/fish/vendor_completions.d/subraumctl.fish
            '';

            meta = {
              description = "MQTT CLI/TUI for the subraum";
              mainProgram = "subraumctl";
            };
          } // extraArgs);

          subraumctl = mkPackage { };
          subraumctl-mcp = mkPackage {
            cargoArtifacts = mcpArtifacts;
            cargoExtraArgs = mcpArgs.cargoExtraArgs;
          };
        in
        {
          packages.default = subraumctl;
          packages.subraumctl = subraumctl;
          packages.subraumctl-mcp = subraumctl-mcp;

          apps.default = {
            type = "app";
            program = "${subraumctl}/bin/subraumctl";
            meta.description = subraumctl.meta.description;
          };

          checks = {
            inherit subraumctl subraumctl-mcp;

            tests = craneLib.cargoTest (commonArgs // {
              inherit cargoArtifacts;
            });

            clippy = craneLib.cargoClippy (commonArgs // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--all-targets -- -D warnings";
            });

            mcp-tests = craneLib.cargoTest (mcpArgs // {
              cargoArtifacts = mcpArtifacts;
            });

            mcp-clippy = craneLib.cargoClippy (mcpArgs // {
              cargoArtifacts = mcpArtifacts;
              cargoClippyExtraArgs = "--all-targets -- -D warnings";
            });
          };

          devShells.default = pkgs.mkShell {
            inputsFrom = [ subraumctl ];
            packages = with pkgs; [
              cargo
              clippy
              rust-analyzer
              rustfmt
              cmake
              pkg-config
              openssl
            ];
          };

          treefmt = {
            projectRootFile = "flake.nix";
            programs = {
              rustfmt.enable = true;
              nixpkgs-fmt.enable = true;
            };
          };
        };
    };
}
