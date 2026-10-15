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
        version = "0.2.0";
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
        # The engraver's spacing is checked against LilyPond's.
        nativeCheckInputs = [ pkgs.lilypond ];
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
      # The AppImage runtime (AppImage/type2-runtime), pinned.
      appimageRuntime = pkgs: pkgs.fetchurl {
        url = "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64";
        hash = "sha256-L8qLRDySUQ8Ug6iD9gBhrQm0a5eLJjHIB82HOkfsJg0=";
      };

      # A portable build for other distributions: compiled with cargo-zigbuild
      # against glibc 2.31 (Ubuntu 20.04, Debian 11 and newer), SQLite
      # compiled in, and only the system's libpipewire linked. Graphics
      # (Vulkan), Wayland/X11 and the file-dialog portal are loaded from the
      # system at run time, as for any desktop app.
      appimage = pkgs: let
        glibc = "2.31";
        target = "x86_64-unknown-linux-gnu";
      in pkgs.rustPlatform.buildRustPackage {
        pname = "corinzia-appimage";
        version = "0.2.0";
        src = self;
        cargoRoot = "rust";
        buildAndTestSubdir = "rust";
        cargoLock.lockFile = ./rust/Cargo.lock;
        nativeBuildInputs = [ pkgs.pkg-config pkgs.rustPlatform.bindgenHook pkgs.cargo-zigbuild pkgs.zig pkgs.squashfsTools ];
        # Headers and a libpipewire to link against; the system's is used at run time.
        buildInputs = [ pkgs.pipewire ];
        doCheck = false; # the regular package runs the tests
        dontFixup = true; # keep the binary free of /nix/store paths
        buildPhase = ''
          runHook preBuild
          export HOME=$TMPDIR ZIG_GLOBAL_CACHE_DIR=$TMPDIR/zig-cache
          cd rust
          # libpipewire from Nix refers to a newer glibc than the target's;
          # the system's own libpipewire is the one loaded at run time.
          RUSTFLAGS="-C link-arg=-Wl,--allow-shlib-undefined" \
            cargo zigbuild --offline --release -p corinzia-app --features portable --target ${target}.${glibc}
          zig cc -shared -fPIC -O2 -target x86_64-linux-gnu.${glibc} -Wl,-soname,libpipewire-0.3.so.0 \
            -o libpipewire-0.3.so.0 ../packaging/pipewire-stub.c
          cd ..
          runHook postBuild
        '';
        installPhase = ''
          runHook preInstall
          app=$TMPDIR/Corinzia.AppDir
          mkdir -p $app/usr/bin $app/usr/lib/no-pipewire $app/usr/share/corinzia \
            $app/usr/share/applications $app/usr/share/icons/hicolor/scalable/apps
          install -m755 rust/target/${target}/release/corinzia $app/usr/bin/corinzia
          install -m755 rust/libpipewire-0.3.so.0 $app/usr/lib/no-pipewire/
          install -m644 ${textFont pkgs} $app/usr/share/corinzia/DejaVuSerif.ttf
          install -m644 ${pkgs.dejavu_fonts}/share/fonts/truetype/LICENSE $app/usr/share/corinzia/DejaVu-LICENSE 2>/dev/null || true
          install -m755 packaging/AppRun $app/AppRun
          install -m644 packaging/corinzia.desktop $app/corinzia.desktop
          install -m644 packaging/corinzia.desktop $app/usr/share/applications/
          install -m644 packaging/corinzia.svg $app/corinzia.svg
          install -m644 packaging/corinzia.svg $app/usr/share/icons/hicolor/scalable/apps/
          ln -s corinzia.svg $app/.DirIcon
          mksquashfs $app $TMPDIR/app.squashfs -root-owned -noappend -comp zstd -quiet
          mkdir -p $out
          cat ${appimageRuntime pkgs} $TMPDIR/app.squashfs > $out/Corinzia-x86_64.AppImage
          chmod 755 $out/Corinzia-x86_64.AppImage
          runHook postInstall
        '';
      };
      # Windows (x86-64): a zip with corinzia.exe and its fonts. Built with
      # the official Rust 1.95 binaries that nixpkgs carries for
      # bootstrapping, the matching official Windows standard library, and
      # nixpkgs' mingw-w64 GCC as linker. Audio through WASAPI.
      windows = pkgs: let
        prebuilt = pkgs.rust.packages.prebuilt;
        mingw = pkgs.pkgsCross.mingwW64;
        target = "x86_64-pc-windows-gnu";
        rustStd = pkgs.fetchurl {
          url = "https://static.rust-lang.org/dist/rust-std-${prebuilt.rustc.version}-${target}.tar.xz";
          hash = "sha256-9X4EUBagQTASX7QyldlfmtK+vClhUOrbAx2/UWetEr0=";
        };
        # The compiler's own sysroot plus the Windows standard library.
        sysroot = pkgs.runCommand "rust-sysroot-windows" { nativeBuildInputs = [ prebuilt.rustc ]; } ''
          mkdir -p $out/lib/rustlib
          for d in $(rustc --print sysroot)/lib/rustlib/*; do ln -s $d $out/lib/rustlib/; done
          tar -xJf ${rustStd}
          cp -r rust-std-*/rust-std-${target}/lib/rustlib/${target} $out/lib/rustlib/
        '';
        rustPlatform = pkgs.makeRustPlatform { rustc = prebuilt.rustc; cargo = prebuilt.cargo; };
        fonts = "${pkgs.dejavu_fonts}/share/fonts/truetype";
      in rustPlatform.buildRustPackage {
        pname = "corinzia-windows";
        version = "0.2.0";
        src = self;
        cargoRoot = "rust";
        buildAndTestSubdir = "rust";
        cargoLock.lockFile = ./rust/Cargo.lock;
        nativeBuildInputs = [ mingw.stdenv.cc pkgs.zip ];
        doCheck = false; # the Linux package runs the tests
        dontFixup = true;
        buildPhase = ''
          runHook preBuild
          cd rust
          export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
          export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
          # Static C runtime; winpthreads for the standard library.
          export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="--sysroot ${sysroot} -C target-feature=+crt-static -L native=${mingw.windows.pthreads}/lib"
          cargo build --offline --release -p corinzia-app --features portable --target ${target}
          cd ..
          runHook postBuild
        '';
        installPhase = ''
          runHook preInstall
          dir=$TMPDIR/Corinzia
          mkdir -p $dir/fonts $out
          x86_64-w64-mingw32-strip -o $dir/corinzia.exe rust/target/${target}/release/corinzia.exe
          for f in DejaVuSerif DejaVuSerif-Bold DejaVuSans DejaVuSans-Bold; do cp ${fonts}/$f.ttf $dir/fonts/; done
          cp packaging/windows/README.txt $dir/README.txt
          (cd $TMPDIR && zip -qr9 $out/Corinzia-x86_64-windows.zip Corinzia)
          runHook postInstall
        '';
      };
    in {
      packages = forAll (pkgs: { default = corinzia pkgs; }
        // pkgs.lib.optionalAttrs (pkgs.stdenv.hostPlatform.system == "x86_64-linux") {
          appimage = appimage pkgs;
          windows = windows pkgs;
          # Both downloads in one ./result: `nix build .#release`.
          release = pkgs.symlinkJoin { name = "corinzia-release-0.2.0"; paths = [ (appimage pkgs) (windows pkgs) ]; };
        });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ cargo rustc clippy rustfmt pkg-config rustPlatform.bindgenHook ];
          buildInputs = with pkgs; [ sqlite pipewire ];
          # xvfb-run + mesa: headless screenshots (scripts/screenshot.sh);
          # wireplumber: the live PipeWire test (scripts/run_with_pipewire.sh);
          # lilypond: the reference for the engraver's spacing tests, and
          # musicxml2ly, an independent check of MusicXML export.
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
