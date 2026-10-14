{
  description = "Corinzia - music notation editor and CLAP DAW for Linux";

  # Pinned to the NixOS stable channel. The channel tarball is used instead of
  # github:NixOS/nixpkgs so the flake also resolves behind proxies that block
  # GitHub archive downloads.
  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-26.05/nixexprs.tar.xz";

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      # What the app (winit + wgpu) loads at runtime.
      guiLibs = pkgs: with pkgs; [ libxkbcommon vulkan-loader wayland libx11 libxcursor libxrandr libxi ];

      # The serif text font of the score (the engraver measures and outlines text with it).
      textFont = pkgs: "${pkgs.dejavu_fonts}/share/fonts/truetype/DejaVuSerif.ttf";

      corinzia = pkgs: pkgs.rustPlatform.buildRustPackage {
        pname = "corinzia";
        version = "0.1.0";
        src = self;
        cargoRoot = "rust";
        buildAndTestSubdir = "rust";
        cargoLock.lockFile = ./rust/Cargo.lock;
        # The app, and the built-in instrument as a standalone CLAP bundle.
        cargoBuildFlags = [ "-p" "corinzia-app" "-p" "corinzia-sine" ];
        # Every crate's tests. The live PipeWire test is opt-in
        # (CORINZIA_PW=1, see scripts/run_with_pipewire.sh).
        cargoTestFlags = [ "--workspace" ];
        # bindgenHook: libclang for the PipeWire bindings (pipewire-rs).
        nativeBuildInputs = [ pkgs.pkg-config pkgs.makeWrapper pkgs.rustPlatform.bindgenHook ];
        buildInputs = [ pkgs.sqlite pkgs.pipewire ];
        # The engraving tests measure text with the real font.
        preCheck = ''
          export CORINZIA_TEXT_FONT=${textFont pkgs}
        '';
        postInstall = ''
          # The built-in instrument as a regular CLAP bundle, for other hosts too.
          mkdir -p $out/lib/clap
          sine=$(find $out/lib target -name libcorinzia_sine.so | head -n1)
          install -m755 "$sine" "$out/lib/clap/Corinzia Sine.clap"
          rm -f $out/lib/libcorinzia_sine.so
          wrapProgram $out/bin/corinzia \
            --prefix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath (guiLibs pkgs)} \
            --set-default CORINZIA_TEXT_FONT ${textFont pkgs}
        '';
        meta.mainProgram = "corinzia";
      };
    in {
      packages = forAll (pkgs: { default = corinzia pkgs; });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ cargo rustc clippy rustfmt pkg-config rustPlatform.bindgenHook ];
          buildInputs = with pkgs; [ sqlite pipewire ];
          # xvfb-run + mesa: headless screenshots (scripts/screenshot.sh);
          # wireplumber: the live PipeWire test (scripts/run_with_pipewire.sh);
          # lilypond: musicxml2ly, an independent check of MusicXML export.
          packages = with pkgs; [ sqlite-interactive xvfb-run mesa wireplumber lilypond ] ++ guiLibs pkgs;
          shellHook = ''
            export CORINZIA_TEXT_FONT=${textFont pkgs}
            # The app (cargo run -p corinzia-app) dlopens these.
            export LD_LIBRARY_PATH=${pkgs.lib.makeLibraryPath (guiLibs pkgs)}''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
            # Software Vulkan (Mesa lavapipe) for headless runs: scripts/screenshot.sh
            export CORINZIA_LAVAPIPE_ICD=${pkgs.mesa}/share/vulkan/icd.d/lvp_icd.x86_64.json
          '';
        };
      });

      apps = forAll (pkgs: {
        default = { type = "app"; program = "${self.packages.${pkgs.system}.default}/bin/corinzia"; };
      });

      checks = forAll (pkgs: { default = self.packages.${pkgs.system}.default; });
    };
}
