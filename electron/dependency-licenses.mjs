import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, lstatSync, statSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';

const root = path.resolve(import.meta.dirname, '..');
export function stageDependencyLicenses(destination) {
  const records = [];
  function notices(kind, name, version, directory, license) {
    const target = `${kind}/${name.replaceAll('/', '__')}-${version}`;
    const output = path.join(destination, 'dependencies', target);
    mkdirSync(output, { recursive: true });
    const files = [];
    function copyNotice(filename) {
      const source = path.join(directory, filename);
      if (lstatSync(source).isSymbolicLink()) throw new Error(`Dependency notice may not be a symlink: ${name}`);
      if (statSync(source).isDirectory()) {
        for (const child of readdirSync(source)) copyNotice(path.posix.join(filename, child));
        return;
      }
      if (!statSync(source).isFile()) return;
      if (files.length >= 100) throw new Error(`Too many license files: ${name}`);
      if (statSync(source).size > 1024 * 1024) throw new Error(`Oversized license notice: ${name}`);
      mkdirSync(path.dirname(path.join(output, filename)), { recursive: true });
      copyFileSync(source, path.join(output, filename));
      files.push({ path: `dependencies/${target}/${filename}`, sha256: createHash('sha256').update(readFileSync(source)).digest('hex') });
    }
    for (const filename of readdirSync(directory).filter(name => /^(?:licen[cs]es?|copying|notice|copyright)(?:[._-]|$)/i.test(name))) copyNotice(filename);
    if (!files.length) {
      const fallback = directory.startsWith(path.join(root, 'crates') + path.sep)
        ? path.join(root, 'LICENSE')
        : path.join(root, 'packaging/licenses/dependencies', `${name}-${version}.LICENSE`);
      if (!existsSync(fallback)) throw new Error(`Missing dependency redistribution notice: ${kind} ${name}@${version}`);
      copyFileSync(fallback, path.join(output, 'LICENSE'));
      files.push({ path: `dependencies/${target}/LICENSE`, sha256: createHash('sha256').update(readFileSync(fallback)).digest('hex') });
    }
    records.push({ ecosystem: kind, name, version, license: license || 'Application license; see module provenance', notices: files });
  }
  const lock = JSON.parse(readFileSync(path.join(root, 'package-lock.json'))).packages;
  const queue = ['@msgpack/msgpack', '@pixi/unsafe-eval', 'pixi.js', 'pixi-live2d-display', 'dompurify', 'highlight.js', 'katex', 'marked'].map(name => `node_modules/${name}`);
  const seen = new Set();
  while (queue.length) {
    const key = queue.shift();
    if (seen.has(key)) continue;
    seen.add(key);
    const packageFile = path.join(root, key, 'package.json');
    if (!existsSync(packageFile)) throw new Error(`Missing locked JavaScript package: ${key}`);
    const pkg = JSON.parse(readFileSync(packageFile));
    notices('npm', pkg.name, pkg.version, path.dirname(packageFile), pkg.license);
    for (const name of Object.keys(lock[key]?.dependencies || {})) {
      let parent = key;
      while (true) {
        const candidate = `${parent ? parent + '/' : ''}node_modules/${name}`;
        if (lock[candidate]) { queue.push(candidate); break; }
        if (!parent) throw new Error(`Cannot resolve locked license dependency: ${name}`);
        parent = parent.replace(/(?:^|\/)node_modules\/(?:@[^/]+\/)?[^/]+$/, '');
      }
    }
  }
  const metadata = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--offline', '--filter-platform', 'x86_64-unknown-linux-gnu', '--format-version', '1'], { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 }));
  for (const pkg of metadata.packages) notices('cargo', pkg.name, pkg.version, path.dirname(pkg.manifest_path), pkg.license);
  writeFileSync(path.join(destination, 'DEPENDENCY_LICENSES.json'), JSON.stringify({ schemaVersion: 1, scope: 'Locked Rust workspace build inputs and bundled JavaScript dependency graph; conservative inventory includes build-only/transitive packages. Electron, native libraries and models have separate notices.', packages: records }, null, 2));
  return records.length;
}
