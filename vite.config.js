import { defineConfig } from 'vite';
import { createReadStream, existsSync, realpathSync, statSync } from 'node:fs';
import { extname, resolve, sep } from 'node:path';

const MODEL_MIME_TYPES = new Map([
  ['.json', 'application/json; charset=utf-8'],
  ['.moc3', 'application/octet-stream'],
  ['.png', 'image/png'],
]);

function localMk6Model() {
  const directory = resolve(process.env.AYLA_LIVE2D_MODEL_DIR || 'Mk6_1.0');
  return {
    name: 'ayla-local-mk6-model',
    configureServer(server) {
      server.middlewares.use('/__ayla_model__', (request, response) => {
        let pathname;
        try {
          pathname = decodeURIComponent(new URL(request.url || '/', 'http://localhost').pathname);
        } catch {
          response.statusCode = 400;
          response.end('Bad model path');
          return;
        }
        let candidate;
        try {
          const root = realpathSync(directory);
          candidate = realpathSync(resolve(root, pathname.replace(/^\/+/, '')));
          if (candidate !== root && !candidate.startsWith(`${root}${sep}`)) {
            response.statusCode = 403;
            response.end('Forbidden');
            return;
          }
          if (!existsSync(candidate) || !statSync(candidate).isFile()) throw new Error('Missing');
        } catch {
          response.statusCode = 404;
          response.end('Mk6 model file not found');
          return;
        }
        response.statusCode = 200;
        response.setHeader(
          'Content-Type',
          MODEL_MIME_TYPES.get(extname(candidate).toLowerCase()) || 'application/octet-stream',
        );
        response.setHeader('Cache-Control', 'no-cache');
        createReadStream(candidate).pipe(response);
      });
    },
  };
}

export default defineConfig({
  plugins: [localMk6Model()],
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
        rewrite: () => '/healthz'
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
