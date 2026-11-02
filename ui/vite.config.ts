import { svelte } from '@sveltejs/vite-plugin-svelte'
import tailwindcss from '@tailwindcss/vite'
import path from 'node:path'
import { defineConfig } from 'vite'

// Tauri serves this on a fixed port in dev and loads ui/dist in release.
export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  resolve: { alias: { $lib: path.resolve('./src/lib') } },
  clearScreen: false,
  // The logo lives in the repo's assets/ folder, one level up.
  server: { port: 5173, strictPort: true, fs: { allow: ['..'] } },
  build: { target: 'es2022' },
})
