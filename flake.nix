{
  outputs = { nixpkgs, ... }:
    let
      inherit (nixpkgs.lib) genAttrs listToAttrs sourceByRegex;
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
        "armv7l-linux"
        "riscv64-linux"
      ];
      forAllSystems = genAttrs supportedSystems;
      mk =
        {
          systems,
          musl ? true, # Takes the total size of the 4 docker images from 69 MB to 40 MB. Burns several days worth of cycles compiling.
        }:
        rec {
          pkgsPlain = import nixpkgs systems;
          pkgs = if musl then pkgsPlain.pkgsMusl else pkgsPlain;
          main = pkgs.callPackage (
            {
              rustPlatform,
              rdkafka,
              pkg-config,
            }:
            rustPlatform.buildRustPackage {
              pname = "kafka-totop";
              version = "0.1.0";
              src = sourceByRegex ./. [
                ".*rs$"
                "^Cargo\\..*"
              ];
              nativeBuildInputs = [ pkg-config ];
              buildInputs = [ rdkafka ];
              doCheck = false;
              env.CARGO_FEATURE_DYNAMIC_LINKING = "yes";
              cargoLock.lockFile = ./Cargo.lock;
              meta.mainProgram = "totop";
            }
          ) { };
          image = pkgs.buildPackages.dockerTools.streamLayeredImage {
            name = "kafka-totop-${systems.crossSystem or "native"}";
            contents = [ pkgs.cacert ];
            config.Entrypoint = [ (pkgs.lib.getExe main) ];
          };
        };
      mkCross =
        localSystem: crossSystem:
        mk {
          systems = {
            inherit localSystem crossSystem;
          };
        };
    in
    {
      packages = forAllSystems (localSystem: {
        default =
          (mk {
            systems = {
              system = localSystem;
            };
            musl = false;
          }).main;
        static =
          (mk {
            systems = {
              system = localSystem;
            };
          }).main;
        mergedOciDir =
          let
            inherit (nixpkgs.lib) concatStringsSep mapAttrsToList getExe';
            pkgs = import nixpkgs { system = localSystem; };
            images = forAllSystems (crossSystem: (mkCross localSystem crossSystem).image);
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
          map (crossSystem: {
            name = "stream-image-${crossSystem}";
            value = {
              type = "app";
              program = "${(mkCross localSystem crossSystem).image}";
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
            inputsFrom = [
              (mk {
                systems = {
                  inherit system;
                };
              })
            ];
          };
        }
      );
    };
  inputs.nixpkgs.url = "github:jcaesar/fork2pr-nixpkgs/pr-21";
}
