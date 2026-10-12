import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';

// Unit tests run in jsdom with the desktop API mocked (tasks/018.md §29).
export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test/setup.ts'],
    include: ['src/**/*.test.{ts,tsx}'],
  },
});
