import { spawnSync } from 'node:child_process';
import { existsSync, createReadStream } from 'node:fs';
import { cp, mkdir, mkdtemp, readFile, readlink, realpath, rename, rm, symlink, lstat, readdir, writeFile } from 'node:fs/promises';
import { homedir, tmpdir } from 'node:os';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';

const repository = 'https://github.com/mkaydin/Ayla.git';
const quote = value => `'${String(value).replaceAll("'", "'\\''")}'`;
function run(command, args, cwd, env = process.env) {
  const result = spawnSync(command, args, { cwd, env, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with ${result.status}`);
}
function options(argv) {
  const result = {};
  const values = new Set(['--source-dir', '--ref', '--commit', '--prefix', '--integration-root']);
  const flags = new Set(['--offline', '--models', '--rollback', '--uninstall', '--status', '--allow-unsigned', '--help']);
  for (let i = 0; i < argv.length; i++) {
    const key = argv[i];
    if (values.has(key)) {
      if (!argv[i + 1] || argv[i + 1].startsWith('--')) throw new Error(`Missing value for ${key}`);
      result[key.slice(2)] = argv[++i];
    } else if (flags.has(key)) result[key.slice(2)] = true;
    else throw new Error(`Unknown option: ${key}`);
  }
  return result;
}

export async function activatePayload(payload, prefix, version, integrationRoot = homedir()) {
  if (/\r|\n/.test(prefix) || prefix === '/' || path.resolve(prefix) === path.resolve(integrationRoot)) throw new Error('Unsafe installation prefix');
  const executable = path.join(payload, 'ayla');
  // A copied desktop entry must belong to the same managed prefix.
  const desktop = path.join(integrationRoot, '.local', 'share', 'applications', 'ayla.desktop');
  if (existsSync(desktop) && !(await readFile(desktop, 'utf8')).includes(path.join(integrationRoot, '.local', 'bin', 'ayla'))) throw new Error('An unrelated ayla desktop entry already exists');
  if (!existsSync(executable) || !existsSync(path.join(payload, 'resources', 'app.asar'))) throw new Error('Incomplete application payload');
  const hash = createHash('sha256');
  async function hashTree(directory, relative = '') {
    for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      const name = path.posix.join(relative, entry.name);
      const absolute = path.join(directory, entry.name);
      hash.update(name + '\0');
      if (entry.isDirectory()) await hashTree(absolute, name);
      else if (entry.isSymbolicLink()) {
        const target = await readlink(absolute);
        const resolved = path.resolve(directory, target);
        if (path.isAbsolute(target) || !resolved.startsWith(path.resolve(payload) + path.sep)) throw new Error('Payload symlink escapes its installation');
        hash.update(target);
      } else if (entry.isFile()) {
        for await (const chunk of createReadStream(absolute)) hash.update(chunk);
      } else throw new Error('Payload contains a special file');
    }
  }
  await hashTree(payload);
  const digest = hash.digest('hex').slice(0, 12);
  const identifier = `${version}-${digest}`;
  if (!/^[a-zA-Z0-9._-]+$/.test(identifier)) throw new Error('Invalid installation version');
  const versions = path.join(prefix, 'versions');
  await mkdir(versions, { recursive: true });
  const destination = path.join(versions, identifier);
  if (!existsSync(destination)) {
    const stage = await mkdtemp(path.join(versions, '.stage-'));
    try { await cp(payload, stage, { recursive: true, dereference: false, verbatimSymlinks: true }); await rename(stage, destination); }
    finally { await rm(stage, { recursive: true, force: true }); }
  }
  const bin = path.join(integrationRoot, '.local', 'bin');
  const applications = path.join(integrationRoot, '.local', 'share', 'applications');
  const launcher = path.join(bin, 'ayla');
  if (existsSync(launcher)) {
    const expected = `exec ${quote(path.join(prefix, 'current', 'ayla'))} "$@"`;
    if (!(await readFile(launcher, 'utf8')).includes(expected)) throw new Error('An unrelated ayla launcher already exists; choose another integration root');
  }
  const current = path.join(prefix, 'current');
  if (existsSync(current)) {
    const previous = await readlink(current);
    if (!previous.startsWith('versions/') || previous.includes('..')) throw new Error('Current installation points outside managed versions');
    if (previous !== `versions/${identifier}`) {
      await rm(path.join(prefix, 'previous'), { force: true });
      await symlink(previous, path.join(prefix, 'previous'));
    }
  }
  const stagedLink = path.join(prefix, '.current-next');
  await rm(stagedLink, { force: true });
  await symlink(`versions/${identifier}`, stagedLink);
  await rename(stagedLink, current);
  await mkdir(bin, { recursive: true }); await mkdir(applications, { recursive: true });
  await writeFile(launcher, `#!/bin/sh\nexec ${quote(path.join(current, 'ayla'))} "$@"\n`, { mode: 0o755 });
  const desktopQuote = value => `"${value.replaceAll('\\', '\\\\').replaceAll('"', '\\"').replaceAll('`', '\\`').replaceAll('$', '\\$')}"`;
  await writeFile(path.join(applications, 'ayla.desktop'), `[Desktop Entry]\nType=Application\nName=Ayla\nComment=AI coworker with persistent knowledge memory\nExec=${desktopQuote(launcher)}\nIcon=${path.join(current, 'resources', 'assets', 'icon.png')}\nTerminal=false\nCategories=Utility;\n`);
  await writeFile(path.join(prefix, 'installation.json'), JSON.stringify({ format: 1, prefix, version, identifier, launcher, desktop: path.join(applications, 'ayla.desktop') }, null, 2));
  return { destination, launcher, identifier };
}

async function main() {
  const args = options(process.argv.slice(2));
  if (args.help) {
    console.log('Build and install Ayla from source:\n  ./install.sh --ref v0.1.0\n  ./install.sh --commit <full Git commit SHA>\n  ./install.sh --source-dir /path/to/Ayla [--offline] [--models]\n  ./install.sh --status | --rollback | --uninstall\n\nRemote tags must be signed with a trusted key. --allow-unsigned is an explicit development override.\nThe default user installation requires no root access and preserves user data.');
    return;
  }
  if (process.platform !== 'linux' || process.arch !== 'x64') throw new Error('The first release supports Linux x86_64');
  if (args.offline && args.models) throw new Error('Offline installation cannot download models. Run models:setup separately when online.');
  if (process.getuid?.() === 0) throw new Error('Run the user installer as your normal user');
  const prefix = path.resolve(args.prefix || path.join(homedir(), '.local', 'opt', 'ayla'));
  if (prefix === '/' || prefix === homedir()) throw new Error('Unsafe installation prefix');
  const integrationRoot = path.resolve(args['integration-root'] || homedir());
  if (Number(process.versions.node.split('.')[0]) < 22 || (Number(process.versions.node.split('.')[0]) === 22 && Number(process.versions.node.split('.')[1]) < 12)) throw new Error('Node >=22.12 is required');
  await mkdir(prefix, { recursive: true, mode: 0o700 });
  const prefixInfo = await lstat(prefix);
  if (!prefixInfo.isDirectory() || prefixInfo.isSymbolicLink() || prefixInfo.uid !== process.getuid()) throw new Error('Installation prefix must be an ordinary directory owned by this user');
  const lock = path.join(prefix, '.install-lock');
  await mkdir(lock).catch(() => { throw new Error('Another installer is running, or a stale .install-lock needs inspection'); });
  let checkout = null;
  try {
    const manifestPath = path.join(prefix, 'installation.json');
    const installation = existsSync(manifestPath) ? JSON.parse(await readFile(manifestPath, 'utf8')) : null;
    if (args.status) { console.log(installation ? JSON.stringify(installation, null, 2) : 'Ayla is not installed at this prefix'); return; }
    if (args.uninstall) {
      if (!installation || installation.format !== 1 || installation.prefix !== prefix) throw new Error('This prefix is not managed by the Ayla installer');
      for (const link of [installation.launcher, installation.desktop]) {
        // Refuse arbitrary paths from a modified manifest.
        if (![path.join(integrationRoot, '.local', 'bin', 'ayla'), path.join(integrationRoot, '.local', 'share', 'applications', 'ayla.desktop')].includes(link)) throw new Error('Invalid desktop integration path');
        await rm(link, { force: true });
      }
      for (const name of ['versions', 'current', 'previous', 'installation.json']) await rm(path.join(prefix, name), { recursive: true, force: true });
      console.log('Application removed. Configuration, knowledge, models, and user documents were preserved.'); return;
    }
    if (args.rollback) {
      if (!installation) throw new Error('No managed installation exists');
      const previous = await readlink(path.join(prefix, 'previous'));
      if (!previous.startsWith('versions/') || previous.includes('..')) throw new Error('Invalid previous version');
      const current = await readlink(path.join(prefix, 'current'));
      if (!current.startsWith('versions/') || current.includes('..')) throw new Error('Invalid current version');
      const next = path.join(prefix, '.rollback-next');
      await rm(next, { force: true });
      await symlink(previous, next); await rename(next, path.join(prefix, 'current'));
      await rm(path.join(prefix, 'previous'), { force: true });
      await symlink(current, path.join(prefix, 'previous'));
      installation.identifier = path.basename(previous);
      installation.version = installation.identifier.replace(/-[a-f0-9]{12}$/, '');
      await writeFile(manifestPath, JSON.stringify(installation, null, 2));
      console.log(`Activated ${previous}. Database schema rollback is separate from application rollback.`); return;
    }
    let source = args['source-dir'] ? await realpath(args['source-dir']) : null;
    if (!source) {
      if (args.offline) throw new Error('Offline installation requires --source-dir');
      if (!args.ref && !args.commit) throw new Error('Select a released --ref or a full --commit SHA');
      if (args.commit && !/^[a-f0-9]{40}$/i.test(args.commit)) throw new Error('Use a full immutable Git commit SHA');
      checkout = await mkdtemp(path.join(tmpdir(), 'ayla-source-'));
      run('git', ['init', checkout]); run('git', ['remote', 'add', 'origin', repository], checkout);
      run('git', ['fetch', '--depth=1', 'origin', args.commit || `refs/tags/${args.ref}`], checkout);
      run('git', ['checkout', '--detach', 'FETCH_HEAD'], checkout);
      if (!args.commit && !args['allow-unsigned']) {
        run('git', ['fetch', '--depth=1', 'origin', `refs/tags/${args.ref}:refs/tags/${args.ref}`], checkout);
        run('git', ['verify-tag', args.ref], checkout);
      }
      source = checkout;
    }
    run('node', ['scripts/bootstrap-native.mjs', ...(args.offline ? ['--offline'] : [])], source);
    run('node', ['scripts/bootstrap-speech.mjs', ...(args.offline ? ['--offline'] : [])], source);
    run('npm', ['ci', ...(args.offline ? ['--offline'] : [])], source);
    run('npm', ['run', 'check'], source);
    const buildEnv = { ...process.env, PDFIUM_LIB_PATH: path.join(source, 'target', 'native', 'pdfium', 'lib', 'libpdfium.so'), ...(args.offline ? { CARGO_NET_OFFLINE: 'true', npm_config_offline: 'true', AYLA_NATIVE_OFFLINE: '1' } : {}) };
    run('cargo', ['test', '--workspace', '--locked', ...(args.offline ? ['--offline'] : [])], source, buildEnv);
    run('npm', ['run', 'electron:pack'], source, buildEnv);
    const pkg = JSON.parse(await readFile(path.join(source, 'package.json'), 'utf8'));
    if (args.models) run('node', ['scripts/setup-models.mjs'], source);
    const result = await activatePayload(path.join(source, 'release', 'electron', 'linux-unpacked'), prefix, pkg.version, integrationRoot);
    const configDir = path.join(process.env.XDG_CONFIG_HOME || path.join(homedir(), '.config'), 'ayla');
    await mkdir(configDir, { recursive: true });
    const config = path.join(configDir, 'settings.env');
    if (!existsSync(config)) {
      const localConfig = args['source-dir'] && existsSync(path.join(source, '.env')) ? path.join(source, '.env') : path.join(source, '.env.example');
      await writeFile(config, await readFile(localConfig), { mode: 0o600 });
    }
    console.log(`Installed ${result.identifier}. Launch ${result.launcher}. Configure local services in ${config}.`);
  } finally {
    if (checkout) await rm(checkout, { recursive: true, force: true });
    await rm(lock, { recursive: true, force: true });
  }
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
