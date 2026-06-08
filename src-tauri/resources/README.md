# Bundled runtime assets

This directory holds the native assets a packaged build ships so PDF preview,
PDF content search, and OCR work with nothing installed on the host:

```
resources/
├── libpdfium.so            # PDF preview (pdfium-render binds to it)
├── tessdata/eng.traineddata # OCR language data (leptess / tesseract)
├── bin/pdftotext           # PDF text extraction (poppler)
├── bin/pdftoppm            # PDF page rasterisation for OCR fallback (poppler)
└── lib/                    # shared libraries the poppler tools link against
```

These files are **not committed**. The release workflow
(`.github/workflows/release.yml`) downloads and populates them, then builds with
the bundle overlay:

```bash
bun tauri build -- --config tauri.bundle.conf.json
```

A plain `bun tauri build` (no overlay) produces a build with no bundled assets;
it relies on the system pdfium, poppler, and tesseract instead. At runtime
`src/runtime_assets.rs` prefers a bundled file when present and falls back to the
system otherwise, so both build modes work.
