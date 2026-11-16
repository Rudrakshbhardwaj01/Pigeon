// Removes the `.gitignore` that wasm-pack generates inside the WASM package
// (it contains `*`, which would prevent the compiled engine from being
// committed). The engine is intentionally tracked so a fresh clone can run
// `npm install && npm run dev` without the Rust toolchain.
const { rmSync } = require("node:fs");
const { join } = require("node:path");

const target = join(__dirname, "..", "apps", "web", "src", "wasm", "pigeon-wasm", ".gitignore");
rmSync(target, { force: true });
console.log("removed apps/web/src/wasm/pigeon-wasm/.gitignore (if present)");
