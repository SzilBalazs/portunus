{ lib
, rustPlatform
, cargo-tauri
, bun
, bun2nix
, pkg-config
, gtk3
, libsoup_3
, webkitgtk_4_1
, gtk-layer-shell
, tesseract
, leptonica
}:

rustPlatform.buildRustPackage {
  pname = "portunus-unwrapped";
  version = (lib.importTOML ../../src-tauri/Cargo.toml).package.version;

  src = ../../.;

  cargoRoot = "src-tauri";
  buildAndTestSubdir = "src-tauri";
  cargoHash = "sha256-oGa0K8493xLF9Qhjq3tdyVDmDfxZUNdDQcxaoEXW0KA=";

  bunDeps = bun2nix.fetchBunDeps { bunNix = ../../bun.nix; };
  dontUseBunBuild = true; # cargo-tauri.hook owns build/install
  dontUseBunCheck = true;
  dontUseBunInstall = true;

  nativeBuildInputs = [
    cargo-tauri.hook
    bun2nix.hook
    bun
    pkg-config
    rustPlatform.bindgenHook # leptess' *-sys crates need libclang for bindgen
  ];

  buildInputs = [
    gtk3
    libsoup_3
    webkitgtk_4_1
    gtk-layer-shell
    tesseract
    leptonica
  ];

  doCheck = true;

  meta = {
    description = "Application launcher and power-user search for Wayland";
    homepage = "https://github.com/SzilBalazs/portunus";
    license = lib.licenses.asl20;
    platforms = lib.platforms.linux;
    mainProgram = "portunus";
  };
}
