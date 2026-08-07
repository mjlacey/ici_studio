/// <reference types="vite/client" />

/** Injected by vite.config.ts's `define` block -- build-time constants for §12.3's run-report provenance. */
interface ImportMetaEnv {
  readonly VITE_APP_VERSION: string;
  readonly VITE_GIT_COMMIT: string;
}
