# Final v1.3.0 JXL color-provenance regression fixtures

Artificial 4 x 1, 8-bit samples. No photographs or personal sample files were used.

- `gray-enumerated.jxl`: PNG with gray samples 30, 90, 150, 220 and no ICC, encoded losslessly by the bundled ImageMagick.
- `rgb-enumerated.jxl`: artificial RGB samples and no ICC, same lossless encoder.
- `gray-embedded.jxl`: losslessly encoded existing `icc-r3/gray-gray.png`, with the exact custom Gray ICC retained.

Fixture provenance was independently checked using the bundled libjxl `JxlDecoderGetColorAsEncodedProfile(dec, ORIGINAL, NULL)` at COLOR_ENCODING. The first two return SUCCESS (structured color definition, decoder synthesizes an ICC); the last returns ERROR (embedded ICC) and the application test separately verifies exact ICC bytes. Results/hashes: `work/bugfix-20261110-final/jxl-provenance.json`. Generation/diagnostic script uses only Python's standard library and the existing DLL; application/tests add no FFI or dependencies.

The bundled ImageMagick coder exposes these representations identically as `icc`, so production does not claim to know JXL's embedded-vs-synthesized provenance. Non-RGB WebP remains safely rejected, with accurate guidance and unchanged pixel/ICC validation.
