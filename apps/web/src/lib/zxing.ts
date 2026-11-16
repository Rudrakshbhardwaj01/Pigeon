// Shared ZXing module configuration for workers.
//
// By default zxing-wasm fetches its .wasm binary from a CDN — which would
// break the offline guarantee and the CSP. We import the binary through the
// bundler (`?url` emits it as a hashed asset, precached by the service
// worker) and point the module at it.

import { prepareZXingModule } from "zxing-wasm/reader";
import readerWasmUrl from "zxing-wasm/reader/zxing_reader.wasm?url";

let prepared = false;

export function selfHostZXing(): void {
  if (prepared) {
    return;
  }
  prepared = true;
  prepareZXingModule({
    overrides: {
      locateFile: () => readerWasmUrl,
    },
    fireImmediately: true,
  });
}

export { readBarcodes } from "zxing-wasm/reader";
