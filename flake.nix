{
  outputs =
    { nixpkgs, ... }:
    let
      inherit (nixpkgs.lib) genAttrs listToAttrs sourceByRegex;
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "armv7l-linux"
        "riscv64-linux"
      ];
      forAllSystems = genAttrs supportedSystems;
      mk = localSystem: crossSystem: rec {
        npkgs = import nixpkgs { system = localSystem; };
        nixpkgsPatched = npkgs.applyPatches {
          name = "nixpkgs-patched";
          src = nixpkgs;
          patches = [ ./libgcc-strip-debug.patch ];
        };
        pkgs =
          (
            if localSystem == crossSystem then
              import nixpkgs { system = localSystem; }
            else
              import nixpkgsPatched { inherit crossSystem localSystem; }
          ).pkgsMusl;
        main = pkgs.callPackage (
          {
            rustPlatform,
            rdkafka,
            buildPackages,
          }:
          rustPlatform.buildRustPackage {
            pname = "kafka-totop";
            version = "0.1.0";
            src = sourceByRegex ./. [
              ".*rs$"
              "^Cargo\\..*"
            ];
            nativeBuildInputs = [ buildPackages.pkg-config ];
            buildInputs = [ rdkafka ];
            doCheck = false;
            env.CARGO_FEATURE_DYNAMIC_LINKING = "yes";
            cargoLock.lockFile = ./Cargo.lock;
            meta.mainProgram = "totop";
          }
        ) { };
        image = pkgs.buildPackages.dockerTools.streamLayeredImage {
          name = "kafka-totop";
          contents = [ pkgs.cacert ];
          config.Entrypoint = [ (pkgs.lib.getExe main) ];
        };
      };
    in
    {
      packages = forAllSystems (localSystem: {
        default = (mk localSystem localSystem).main;
        mergedOciDir =
          let
            inherit (nixpkgs.lib) concatStringsSep mapAttrsToList getExe';
            pkgs = import nixpkgs { system = localSystem; };
            images = forAllSystems (crossSystem: (mk localSystem crossSystem).image);
            regctl = getExe' pkgs.regctl "regctl";
            ociDirs = mapAttrsToList (k: v: ''
              ${v} >docker-${k}.tar
              ${regctl} image import ocidir://oci-${k} docker-${k}.tar
              rm docker-${k}.tar
            '') images;
            mergeRefs = mapAttrsToList (k: _: "--ref ocidir://oci-${k}") images;
            env.nativeBuildInputs = [ pkgs.regctl ];
            cmd = ''
              ${concatStringsSep "\n" ociDirs}

              ${regctl} index create ocidir://$out \
                ${concatStringsSep " " mergeRefs}

              echo You can now run: ${regctl} image copy ocidir://$out docker.io/foobar/kafka-totop
            '';
          in
          pkgs.runCommand "merge-images" env cmd;
      });
      apps = forAllSystems (
        localSystem:
        listToAttrs (
          map (remoteSystem: {
            name = "stream-image-${remoteSystem}";
            value = {
              type = "app";
              program = "${(mk localSystem remoteSystem).image}";
            };
          }) supportedSystems
        )
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            buildInputs = with pkgs; [
              cargo
              rustfmt
              rust-analyzer
              cargo-watch
              rustc
            ];
            inputsFrom = [ (mk system system) ];
          };
        }
      );
    };
}
