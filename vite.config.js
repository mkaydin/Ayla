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
      },
      '/api/services/ollama': {
        target: 'http://127.0.0.1:11434',
        changeOrigin: true,
        rewrite: () => '/api/tags'
      },
      '/api/services/surrealdb': {
        target: 'http://127.0.0.1:8000',
        changeOrigin: true,
        rewrite: () => '/health'
      },
      '/api/services/qdrant': {
        target: 'http://127.0.0.1:6333',
        changeOrigin: true,
        rewrite: () => '/'
      },
      '/api/services/searxng': {
        target: 'http://127.0.0.1:8888',
        changeOrigin: true,
        rewrite: () => '/search?q=test&format=json'
      },
      '/api/services/lightpanda': {
        target: 'http://127.0.0.1:9222',
        changeOrigin: true,
        rewrite: () => '/json/version'
      }
    }
  },
  // pixi-live2d-display needs window globals; vite optimize deps
  optimizeDeps: {
    include: ['pixi.js', 'pixi-live2d-display']
  }
});
