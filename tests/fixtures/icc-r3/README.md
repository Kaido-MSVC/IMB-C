# R3 ICC regression fixtures

All PNGs are 4 x 1, 8-bit neutral samples (30, 90, 150, 220).

- `rgb-existing.icc`: exact RGB profile extracted from the existing `acceptance-v13/icc8.png` fixture (4324 bytes). This is the existing Rec2100PQ RGB test profile, not a new claim of SDR/sRGB appearance.
- `gray-gamma22.icc`: 348-byte Gray/D50/gamma-2.2 profile generated locally using the already bundled LittleCMS `cmsBuildGamma`, `cmsD50_xyY`, `cmsCreateGrayProfile`, and `cmsSaveProfileToMem`. Tests use the fixed bytes and need no additional package or FFI.
- `rgb-rgb.png`: truecolor PNG with the RGB ICC.
- `gray-rgb.png`: grayscale-storage PNG written by the bundled ImageMagick from `rgb-rgb.png`. It contains iCCP and the engine's legacy `Raw profile type icc` zTXt representation. Full reading recovers the original ICC; ping misses it. This reproduces the real round-2 regression.
- `gray-gray.png`: grayscale PNG with the Gray ICC.
- `unreadable-gray-iccp.png`: grayscale PNG with only the native RGB iCCP, without the legacy raw-profile representation. The bundled libpng refuses that incompatible iCCP. It is intentionally a negative case and must not be silently treated as untagged.

PNG compression and CRC construction used Python standard-library zlib/struct. No test dependency or runtime was installed. Source fixtures and profiles are preserved byte-for-byte by the tests; PNG/WebP/JXL output ICC is compared independently to the exact expected bytes, or the output must be refused.
