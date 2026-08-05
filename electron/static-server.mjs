import { createReadStream, realpathSync, statSync } from 'node:fs';
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
    "connect-src 'self' blob: http://localhost:* http://127.0.0.1:* ws://localhost:* ws://127.0.0.1:*",
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
  ['searxng', { port: 8888, path: '/healthz' }],
  ['lightpanda', { port: 9222, path: '/json/version' }],
]);

function safeFilePath(root, requestUrl) {
  const pathname = decodeURIComponent(new URL(requestUrl, 'http://127.0.0.1').pathname);
  const relativePath = pathname === '/' ? 'index.html' : pathname.replace(/^\/+/, '');
  const candidate = resolve(root, relativePath);
  return candidate === root || candidate.startsWith(`${root}${sep}`) ? candidate : null;
}

function safeModelFilePath(root, requestUrl) {
  const canonicalRoot = realpathSync(root);
  const candidate = safeFilePath(canonicalRoot, requestUrl);
  if (!candidate) return null;
  let canonicalFile;
  try {
    canonicalFile = realpathSync(candidate);
  } catch {
    return candidate;
  }
  return canonicalFile === canonicalRoot || canonicalFile.startsWith(`${canonicalRoot}${sep}`)
    ? canonicalFile
    : null;
}

function serveFile(response, filePath, { immutable = true } = {}) {
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
    'Cache-Control': immutable ? 'public, max-age=31536000, immutable' : 'no-cache',
    ...SECURITY_HEADERS,
  });
  createReadStream(filePath).pipe(response);
}

export async function startStaticServer(distDirectory, { modelDirectory = null } = {}) {
  const root = resolve(distDirectory);
  const modelRoot = modelDirectory ? resolve(modelDirectory) : null;
  const server = createServer((request, response) => {
    if (request.url?.startsWith('/api/ollama/')) {
      const upstreamPath = request.url.replace(/^\/api\/ollama/, '') || '/';
      proxyRequest(request, response, 11434, upstreamPath);
      return;
    }

    if (request.url?.startsWith('/__alice_model__/')) {
      if (!modelRoot) {
        response.writeHead(404, SECURITY_HEADERS);
        response.end('Mk6 model directory is not configured');
        return;
      }
      const modelUrl = request.url.replace(/^\/__alice_model__/, '') || '/';
      let modelPath;
      try {
        modelPath = safeModelFilePath(modelRoot, modelUrl);
      } catch {
        response.writeHead(400, SECURITY_HEADERS);
        response.end('Bad model request');
        return;
      }
      if (!modelPath) {
        response.writeHead(403, SECURITY_HEADERS);
        response.end('Forbidden');
        return;
      }
      serveFile(response, modelPath, { immutable: false });
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

    serveFile(response, filePath, { immutable: !filePath.endsWith('index.html') });
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
