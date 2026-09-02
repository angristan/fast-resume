{
  lib,
  rustPlatform,
  versionCheckHook,
}:
let
  manifest = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = manifest.package.name;
  inherit (manifest.package) version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
      ../tests
      ../assets
      ../skills
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  postInstall = ''
    ln -s fr "$out/bin/fast-resume"
  '';

  doInstallCheck = true;
  nativeInstallCheckInputs = [ versionCheckHook ];
  versionCheckProgramArg = "--version";

  meta = {
    description = manifest.package.description;
    homepage = "https://github.com/angristan/fast-resume";
    changelog = "https://github.com/angristan/fast-resume/blob/v${manifest.package.version}/CHANGELOG.md";
    license = lib.licenses.mit;
    mainProgram = "fr";
    platforms = [
      "aarch64-darwin"
      "aarch64-linux"
      "x86_64-darwin"
      "x86_64-linux"
    ];
  };
}
