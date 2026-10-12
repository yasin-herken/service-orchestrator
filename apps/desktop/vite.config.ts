import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// Vite configuration for the Tauri desktop frontend.
//
// Tauri drives this server in development (`beforeDevCommand`) and consumes
// `dist/` as the frontend asset directory in production builds. The Rust
// sources under src-tauri are excluded from the watcher: its `target/`
// directory churns on every build.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ['**/src-tauri/**'],
    },
  },
});
