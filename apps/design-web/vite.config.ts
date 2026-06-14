import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const apiTarget = process.env.JFC_DESIGN_API ?? 'http://127.0.0.1:4322';

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5174,
    strictPort: false,
    proxy: {
      '/design': {
        target: apiTarget,
        changeOrigin: true
      },
      '/health': {
        target: apiTarget,
        changeOrigin: true
      }
    }
  }
});
