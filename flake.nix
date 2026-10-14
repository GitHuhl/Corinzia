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

      # Unwrapped binaries (dev shell, test phase) need the QML import path that
      # wrapQtAppsHook only adds to installed programs; log to stderr, not journald.
      qtRuntimeEnv = pkgs: ''
        export QML_IMPORT_PATH=${pkgs.qt6.qtdeclarative}/${pkgs.qt6.qtbase.qtQmlPrefix}
        export QT_FORCE_STDERR_LOGGING=1
        export LANG=C.UTF-8
      '';

      # Rust: the workspace in rust/ (export, native engraver) is built by CMake through
      # Corrosion; in the sandbox its crates come from Cargo.lock via
      # cargoSetupHook (vendored, offline).
      nativeDeps = pkgs: with pkgs; [ cmake ninja pkg-config qt6.wrapQtAppsHook cargo rustc corrosion ];
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
          nativeBuildInputs = nativeDeps pkgs ++ [ pkgs.rustPlatform.cargoSetupHook ];
          buildInputs = buildDeps pkgs;
          # Crates for the rust/ workspace, pinned by its Cargo.lock and vendored
          # (the sandbox has no network); Corrosion's cargo build uses them.
          cargoRoot = "rust";
          cargoDeps = pkgs.rustPlatform.importCargoLock { lockFile = ./rust/Cargo.lock; };
          cmakeFlags = [ "-DCORINZIA_BUILD_TESTS=ON" ];
          doCheck = true;
          # Run the LilyPond round-trip and offscreen Qt tests inside the sandbox too.
          nativeCheckInputs = [ pkgs.lilypond pkgs.pipewire ];   # pipewire: private daemon + pw-link for the backend test
          preCheck = ''
            export HOME=$TMPDIR
            # The sandbox has no fontconfig setup: without one LilyPond cannot
            # load even its own text fonts (titles, tempo marks, text vanish).
            export FONTCONFIG_FILE=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }}
            export CORINZIA_LILYPOND=${pkgs.lilypond}/bin/lilypond
            ${qtRuntimeEnv pkgs}
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
            ${qtRuntimeEnv pkgs}
          '';
        };
      });

      apps = forAll (pkgs: {
        default = { type = "app"; program = "${self.packages.${pkgs.system}.default}/bin/corinzia"; };
      });

      checks = forAll (pkgs: { build = self.packages.${pkgs.system}.default; });
    };
}
