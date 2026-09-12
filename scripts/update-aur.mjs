// Generate the binary AUR recipe from the exact reviewed payload checksum.
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '..');
const { version } = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'));
if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Use a stable numeric package version');
const archive = `Ayla-${version}-linux-x64.tar.gz`;
const payload = path.resolve(process.argv[2] || path.join(root, 'release', 'github', archive));
const hash = createHash('sha256');
for await (const bytes of createReadStream(payload)) hash.update(bytes);
const digest = hash.digest('hex');
const directory = path.join(root, 'packaging', 'arch', 'ayla-ai-bin');
await mkdir(directory, { recursive: true });
const recipe = `# Maintainer: Ayla contributors <noreply@localhost>
pkgname=ayla-ai-bin
pkgver=${version}
pkgrel=1
pkgdesc='AI coworker with speech, OCR, research and persistent knowledge'
arch=('x86_64')
url='https://github.com/mkaydin/Ayla'
license=('MIT' 'Apache-2.0' 'GPL-3.0-or-later' 'LicenseRef-OpenRAIL-M')
depends=('glibc>=2.36' 'gtk3' 'nss' 'alsa-lib' 'libxss' 'libxtst' 'libx11' 'libxcomposite' 'libxdamage' 'libxrandr' 'mesa' 'libdrm' 'dbus' 'systemd-libs' 'libxext' 'libxfixes' 'expat' 'cairo' 'at-spi2-core' 'libcups' 'nspr' 'hicolor-icon-theme' 'pango' 'libxkbcommon' 'glib2')
optdepends=('ollama: chat, embeddings and GLM OCR' 'docker: optional storage and search services' 'docker-compose: local service stack')
provides=('ayla')
conflicts=('ayla-ai')
options=('!strip' '!debug')
source=("https://github.com/mkaydin/Ayla/releases/download/v\$pkgver/${archive}")
sha256sums=('${digest}')

package() {
  install -dm755 "\$pkgdir/usr/lib/ayla-ai-bin"
  cp -a --no-preserve=ownership "\$srcdir/ayla-\$pkgver/." "\$pkgdir/usr/lib/ayla-ai-bin/"
  # Arch's packaged Chromium sandbox helper must have its expected permissions.
  chmod 4755 "\$pkgdir/usr/lib/ayla-ai-bin/chrome-sandbox"
  install -dm755 "\$pkgdir/usr/bin" "\$pkgdir/usr/share/applications"
  ln -s /usr/lib/ayla-ai-bin/ayla "\$pkgdir/usr/bin/ayla"
  cat > "\$pkgdir/usr/share/applications/ayla.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Ayla
Exec=ayla
Icon=ayla
Terminal=false
Categories=Utility;
DESKTOP
  install -Dm644 "\$srcdir/ayla-\$pkgver/resources/assets/icon.png" "\$pkgdir/usr/share/icons/hicolor/512x512/apps/ayla.png"
  install -dm755 "\$pkgdir/usr/share/licenses/\$pkgname"
  cp -a --no-preserve=ownership "\$srcdir/ayla-\$pkgver/resources/licenses/." "\$pkgdir/usr/share/licenses/\$pkgname/"
}
`;
await writeFile(path.join(directory, 'PKGBUILD'), recipe);
for (const name of ['ayla-ai', 'ayla-ai-bin']) {
  const dir = path.join(root, 'packaging', 'arch', name);
  const result = spawnSync('makepkg', ['--printsrcinfo'], { cwd: dir, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(`Cannot generate ${name} metadata: ${result.stderr || result.error}`);
  await writeFile(path.join(dir, '.SRCINFO'), result.stdout);
}
console.log(`Generated ayla-ai-bin PKGBUILD/.SRCINFO with reviewed payload SHA256 ${digest}`);
