# v1.3 final acceptance fixtures

These tiny synthetic fixtures were generated locally on 2026-10-29. They contain no user photographs or personal metadata.

- `icc8.png`: truecolor 8-bit version of the existing 16-bit ICC ramp. Used to reproduce ICC-transform precision loss at the output stage.
- `pq-original.jxl`, `hlg-original.jxl`: libjxl 0.12.0 generated 4x1 RGB 16-bit ramps using Rec2100PQ / Rec2100HLG. ICC profiles are embedded.
- `baseline-metadata.jpg`, `progressive-metadata.jpg`: synthetic JPEGs with EXIF orientation 6, a synthetic XMP description, Windows sRGB ICC, and a COM marker. Tests compare the entire JPEG byte sequence after JXL packing and reconstruction, including all markers.

The engine regression test changes the synthetic baseline JPEG's EXIF orientation tag through 1–8 and checks AVIF dimensions and pixel positions. `v13-cicp-only-pq.png` is a 16-bit PQ PNG with a BT.2020/PQ cICP chunk and no source iCCP chunk; conversion must reject it without saving an output.

Run `cargo run --locked --bin acceptance-v13 -- <project-root>` after restoring the private runtime to `target/release`. This uses the same engine module as the application. Output is written under `work/focused`; sources must stay unchanged. The selected target ICC is the existing Windows sRGB profile.

The ordinary pixel converter and the dedicated JPEG reconstruction tool have different guarantees: the latter is checked for original JPEG bytes; pixel conversion is checked for decoded pixels and ICC. No general claim is made for CLLI, mastering-display metadata, gain maps, EXIF, XMP, or container box preservation during ordinary conversion.
