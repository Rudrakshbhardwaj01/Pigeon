# Third-party licenses and attribution

Pigeon's own code is MIT-licensed (`LICENSE`). It depends on the following
third-party components; each is distributed under its own license.

## Rust dependencies (crates)

| Crate | License | Role |
|---|---|---|
| raptorq 2.0.1 | Apache-2.0 | RFC 6330 RaptorQ fountain coding |
| fast_qr 0.14.0 | MIT | QR matrix generation |
| gif 0.14.2 | MIT / Apache-2.0 | GIF encoding and streaming parse |
| crc32c 0.6.9 | BSD-2-Clause / MIT | Castagnoli packet checksums |
| sha2 0.10 | MIT / Apache-2.0 | SHA-256 content identity |
| flate2 1.1 | MIT / Apache-2.0 | optional zlib compression |
| getrandom 0.2 | MIT / Apache-2.0 | CSPRNG for transfer ids |
| serde / serde_derive | MIT / Apache-2.0 | boundary serialization |
| thiserror 2 | MIT / Apache-2.0 | error definitions |
| wasm-bindgen / js-sys | MIT / Apache-2.0 | WASM bindings |
| serde-wasm-bindgen 0.6 | MIT | JS value serialization |
| rqrr 0.11 (dev only) | MIT | QR decoding in native tests |

## Frontend dependencies (npm)

| Package | License | Role |
|---|---|---|
| react / react-dom 19 | MIT | UI |
| lucide-react | ISC | icons |
| zxing-wasm 3.1.6 | MIT; underlying zxing-cpp: Apache-2.0; zint: BSD-3-Clause | QR decoding (self-hosted WASM) |
| vite / @vitejs/plugin-react | MIT | build tooling |
| vite-plugin-pwa + workbox | MIT | service worker / offline cache |
| vitest / @testing-library/* / jsdom | MIT | unit tests |
| @playwright/test | Apache-2.0 | browser end-to-end tests |
| typescript / eslint / typescript-eslint | Apache-2.0 | type checking / linting |

License texts are distributed with the respective packages (`node_modules`,
Cargo registry) and can be enumerated with `npm ls` / the `package-lock.json` and
`cargo license` (if installed).

## Attribution

- The animated-QR + fountain-code transfer concept was popularized by TXQR
  (divan/txqr, MIT). Pigeon is an independent implementation informed by
  studying TXQR, RaptorQR (infrost/RaptorQR, MIT) and AirQR
  (Thunzyy/AirQR, MIT — Copyright (c) 2026 Lucas Poignard); no source code
  was copied from these projects. See `docs/REFERENCE_RESEARCH.md`.
- RaptorQ implements RFC 6330; the `raptorq` crate is by Cyril Plisko /
  cberner and contributors.
- ZXing-C++ is by the ZXing-C++ project ( zxing-cpp/zxing-cpp ).
