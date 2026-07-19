import { defineConfig } from 'vite';

export default defineConfig({
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: false,
    open: false,
    fs: {
      // allow serving files outside project root if needed
      strict: false
    },
    headers: {
      'Cache-Control': 'no-cache'
    },
    proxy: {
      // Proxy Ollama API to avoid CORS issues in the browser/webview.
      // Frontend calls /api/ollama/* → forwarded to http://localhost:11434/*
      '/api/ollama': {
        target: 'http://localhost:11434',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api\/ollama/, '')
      }
    }
  },
  // pixi-live2d-display needs window globals; vite optimize deps
  optimizeDeps: {
    include: ['pixi.js', 'pixi-live2d-display']
  }
});
