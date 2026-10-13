{
  description = "Corinzia - hybrid music notation editor / CLAP DAW for Linux";

  # Pinned to the NixOS stable channel. The channel tarball is used instead of
  # github:NixOS/nixpkgs so the flake also resolves behind proxies that block
  # GitHub archive downloads.
  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      nativeDeps = pkgs: with pkgs; [ cmake ninja pkg-config qt6.wrapQtAppsHook ];
      buildDeps = pkgs: with pkgs; [
        clap                 # CLAP plugin ABI headers
        sqlite               # project container (>= 3.45 for JSONB)
        pipewire             # primary audio backend
        libjack2             # optional JACK backend
        alsa-lib             # fallback backend
        qt6.qtbase
        qt6.qtdeclarative    # Qt Quick / QML / scene graph
        qt6.qtsvg            # QSvgRenderer for SVG id -> bounds mapping
      ];
    in {
      packages = forAll (pkgs: {
        default = pkgs.stdenv.mkDerivation {
          pname = "corinzia";
          version = "0.1.0";
          src = self;
          nativeBuildInputs = nativeDeps pkgs;
          buildInputs = buildDeps pkgs;
          cmakeFlags = [ "-DCORINZIA_BUILD_TESTS=ON" ];
          doCheck = true;
          # Run the LilyPond round-trip and offscreen Qt tests inside the sandbox too.
          nativeCheckInputs = [ pkgs.lilypond ];
          preCheck = ''
            export HOME=$TMPDIR
            export CORINZIA_LILYPOND=${pkgs.lilypond}/bin/lilypond
          '';
          # Engraving backend is an out-of-process tool resolved at runtime.
          qtWrapperArgs = [ "--prefix PATH : ${pkgs.lilypond}/bin" ];
        };
      });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = nativeDeps pkgs;
          buildInputs = buildDeps pkgs;
          packages = with pkgs; [ lilypond sqlite-interactive python3 gdb clang-tools ];
          shellHook = ''
            export CORINZIA_LILYPOND=${pkgs.lilypond}/bin/lilypond
          '';
        };
      });

      checks = forAll (pkgs: { build = self.packages.${pkgs.system}.default; });
    };
}
