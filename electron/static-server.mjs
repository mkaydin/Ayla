import { createReadStream, statSync } from 'node:fs';
import { createServer, request as httpRequest } from 'node:http';
import { extname, resolve, sep } from 'node:path';

const MIME_TYPES = new Map([
  ['.css', 'text/css; charset=utf-8'],
  ['.html', 'text/html; charset=utf-8'],
  ['.ico', 'image/x-icon'],
  ['.jpeg', 'image/jpeg'],
  ['.jpg', 'image/jpeg'],
  ['.js', 'text/javascript; charset=utf-8'],
  ['.json', 'application/json; charset=utf-8'],
  ['.moc3', 'application/octet-stream'],
  ['.png', 'image/png'],
  ['.svg', 'image/svg+xml'],
  ['.wasm', 'application/wasm'],
  ['.wav', 'audio/wav'],
  ['.webp', 'image/webp'],
]);

const SECURITY_HEADERS = {
  'Content-Security-Policy': [
    "default-src 'self'",
    "script-src 'self' https://cubism.live2d.com",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "media-src 'self' data: blob:",
    "connect-src 'self' http://localhost:* http://127.0.0.1:* ws://localhost:* ws://127.0.0.1:*",
    "font-src 'self' data:",
    "worker-src 'self' blob:",
    "object-src 'none'",
    "base-uri 'self'",
    "frame-ancestors 'none'",
  ].join('; '),
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Referrer-Policy': 'no-referrer',
  'X-Content-Type-Options': 'nosniff',
};

function proxyRequest(incoming, response, port, upstreamPath) {
  const upstream = httpRequest({
    hostname: '127.0.0.1',
    port,
    method: incoming.method,
    path: upstreamPath,
    headers: {
      ...incoming.headers,
      host: `127.0.0.1:${port}`,
    },
  }, upstreamResponse => {
    response.writeHead(upstreamResponse.statusCode || 502, upstreamResponse.headers);
    upstreamResponse.pipe(response);
  });

  upstream.setTimeout(10 * 60 * 1000, () => {
    upstream.destroy(new Error('Local service request timed out'));
  });
  upstream.on('error', error => {
    if (!response.headersSent) {
      response.writeHead(502, {
        'Content-Type': 'application/json; charset=utf-8',
        ...SECURITY_HEADERS,
      });
    }
    response.end(JSON.stringify({ error: `Local service is unavailable: ${error.message}` }));
  });
  incoming.pipe(upstream);
}

const SERVICE_HEALTH_TARGETS = new Map([
  ['ollama', { port: 11434, path: '/api/tags' }],
  ['surrealdb', { port: 8000, path: '/health' }],
  ['qdrant', { port: 6333, path: '/' }],
  ['searxng', { port: 8888, path: '/search?q=test&format=json' }],
  ['lightpanda', { port: 9222, path: '/json/version' }],
]);

function safeFilePath(root, requestUrl) {
  const pathname = decodeURIComponent(new URL(requestUrl, 'http://127.0.0.1').pathname);
  const relativePath = pathname === '/' ? 'index.html' : pathname.replace(/^\/+/, '');
  const candidate = resolve(root, relativePath);
  return candidate === root || candidate.startsWith(`${root}${sep}`) ? candidate : null;
}

export async function startStaticServer(distDirectory) {
  const root = resolve(distDirectory);
  const server = createServer((request, response) => {
    if (request.url?.startsWith('/api/ollama/')) {
      const upstreamPath = request.url.replace(/^\/api\/ollama/, '') || '/';
      proxyRequest(request, response, 11434, upstreamPath);
      return;
    }

    const healthMatch = request.url?.match(/^\/api\/services\/([a-z]+)$/);
    if (healthMatch && request.method === 'GET') {
      const target = SERVICE_HEALTH_TARGETS.get(healthMatch[1]);
      if (!target) {
        response.writeHead(404, SECURITY_HEADERS);
        response.end('Unknown service');
        return;
      }
      proxyRequest(request, response, target.port, target.path);
      return;
    }

    let filePath;
    try {
      filePath = safeFilePath(root, request.url || '/');
    } catch {
      response.writeHead(400, SECURITY_HEADERS);
      response.end('Bad request');
      return;
    }

    if (!filePath) {
      response.writeHead(403, SECURITY_HEADERS);
      response.end('Forbidden');
      return;
    }

    try {
      const stats = statSync(filePath);
      if (!stats.isFile()) throw new Error('Not a file');
    } catch {
      response.writeHead(404, SECURITY_HEADERS);
      response.end('Not found');
      return;
    }

    response.writeHead(200, {
      'Content-Type': MIME_TYPES.get(extname(filePath).toLowerCase()) || 'application/octet-stream',
      'Cache-Control': filePath.endsWith('index.html')
        ? 'no-cache'
        : 'public, max-age=31536000, immutable',
      ...SECURITY_HEADERS,
    });
    createReadStream(filePath).pipe(response);
  });

  await new Promise((resolveListen, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolveListen);
  });

  const address = server.address();
  if (!address || typeof address === 'string') {
    server.close();
    throw new Error('Electron static server did not receive a TCP address');
  }

  return {
    origin: `http://127.0.0.1:${address.port}`,
    close: () => new Promise(resolveClose => server.close(resolveClose)),
  };
}
