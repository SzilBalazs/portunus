{ lib
, stdenv
, wrapGAppsHook3
, xorg
, cargo-tauri
, glib-networking
, gst_all_1
, pdfium-binaries
, poppler-utils
, wl-clipboard
, cliphist
, dict
, wtype
, tesseract
, dbus
, unwrapped
}:

stdenv.mkDerivation {
  pname = "portunus";
  inherit (unwrapped) version meta;

  dontUnpack = true;

  nativeBuildInputs = [ wrapGAppsHook3 xorg.lndir ];

  # runtime pieces the hook picks up from buildInputs
  buildInputs = [
    glib-networking
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
  ];

  # Symlink the unwrapped output instead of copying it - the binary and its
  # resources would otherwise sit in the store twice. lndir mirrors the tree as
  # real dirs holding symlinks, so wrapProgram can still replace bin/portunus.
  installPhase = ''
    runHook preInstall
    mkdir -p $out
    lndir -silent ${unwrapped} $out
    runHook postInstall
  '';

  preFixup = ''
    gappsWrapperArgs+=(
      # re-applied from cargo-tauri.hook's fixup for wrapGAppsHook3
      --prefix WEBKIT_GST_ALLOWED_URI_PROTOCOLS : "asset"
      --prefix GST_PLUGIN_SYSTEM_PATH_1_0 : "${cargo-tauri.gst-plugin}/lib/gstreamer-1.0/"
      --set-default __NV_DISABLE_EXPLICIT_SYNC 1

      --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath [ pdfium-binaries ]}
      --prefix PATH : ${lib.makeBinPath [ poppler-utils wl-clipboard cliphist dict wtype tesseract dbus ]}
      # tesseract above is only for the dep detection
      # OCR uses the linked library + this data path.
      --set-default TESSDATA_PREFIX ${tesseract}/share/tessdata
    )
  '';
}
