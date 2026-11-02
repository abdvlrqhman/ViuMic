import { svelte } from '@sveltejs/vite-plugin-svelte'
import { defineConfig } from 'vite'

// Tauri serves this on a fixed port in dev and loads ui/dist in release.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // The logo lives in the repo's assets/ folder, one level up.
  server: { port: 5173, strictPort: true, fs: { allow: ['..'] } },
  build: { target: 'es2022' },
})
