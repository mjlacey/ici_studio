// §12.3's run-report provenance wants app version + git commit -- both are
// build-time facts, not runtime-configurable secrets. Injected via
// `import.meta.env.VITE_*` (Vite's own first-class mechanism for exactly
// this) rather than a bare custom `define` global: confirmed by direct
// testing that a plain `define: { __APP_VERSION__: ... }` global gets
// substituted correctly in `vite build` but is left as an unresolved
// bare identifier (ReferenceError at runtime) by `vite`'s dev server --
// `import.meta.env.VITE_*` doesn't have that dev/build split, since
// Vite's dev server has native handling for `import.meta.env` access.
// Falls back to "unknown" when `git rev-parse` fails (e.g. this repo
// currently has zero commits) rather than failing the build.

import { execSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

const pkg = JSON.parse(readFileSync(fileURLToPath(new URL("./package.json", import.meta.url)), "utf-8")) as { version: string };

function gitCommit(): string {
  try {
    return execSync("git rev-parse --short HEAD", { cwd: fileURLToPath(new URL(".", import.meta.url)) })
      .toString()
      .trim();
  } catch {
    return "unknown";
  }
}

export default defineConfig({
  define: {
    "import.meta.env.VITE_APP_VERSION": JSON.stringify(pkg.version),
    "import.meta.env.VITE_GIT_COMMIT": JSON.stringify(gitCommit()),
  },
});
