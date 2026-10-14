// Public, generated evidence-review fixtures; no private inputs or model calls.
import { app, BrowserWindow } from 'electron';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';

if (app.commandLine.hasSwitch('no-sandbox')) app.exit(64);
const output = path.resolve(process.env.AYLA_OCR_CORPUS_DIR || path.join(import.meta.dirname, '../release/ocr-corpus'));
const style = '<style>body{font:22px "DejaVu Sans",sans-serif;padding:35px;color:#111}h1{font-size:30px}h2{font-size:25px}table{border-collapse:collapse;width:100%}td,th{border:1px solid #222;padding:12px;text-align:left}.columns{display:grid;grid-template-columns:1fr 1fr;gap:35px}.figure{border:2px solid #333;padding:20px;margin-top:25px}small{font-size:18px}</style>';
const fixtures = [
  {
    name: 'bilingual-columns',
    html: '<h1>Public OCR Fixture — Türkçe / English</h1><div class="columns"><section><h2>Türkçe deney</h2><p>İstanbul laboratuvarında ölçülen basınç 42,75 MPa olarak kaydedildi.</p><p>Şişe sıcaklığı 23 °C ve ölçüm süresi 15 saniyedir.</p></section><section><h2>English experiment</h2><p>The measured pressure was 42.75 MPa.</p><p>The specimen temperature was 23 °C and the observation lasted 15 seconds.</p></section></div><p>Sonuç / Conclusion: This is a generated public fixture, not a scientific finding.</p>',
    review: ['Preserve Turkish diacritics and decimal comma.', 'Read each column coherently; do not interleave unrelated sentences.', 'Keep 42.75/42,75 MPa and 23 °C attached to their quantities.'],
    criticalTokens: ['42.75', 'MPa', '23', '15'],
    expectedWords: ['İstanbul', 'Şişe'],
  },
  {
    name: 'table-equation-figure',
    html: '<h1>Public scientific evidence fixture</h1><h2>Calibration table</h2><table><tr><th>Specimen</th><th>Pressure (MPa)</th><th>Temperature (°C)</th></tr><tr><td>A</td><td>42.75</td><td>23</td></tr><tr><td>B</td><td>18.20</td><td>31</td></tr></table><h2>Equation</h2><p>σ = E · ε</p><p>E = 210 GPa; ε = 0.002; σ = 420 MPa.</p><div class="figure">A → sensor → B<br><small>Figure 1. Generated calibration chain; arrows indicate signal direction.</small></div><p>Reference statement: Specimen A has pressure 42.75 MPa at 23 °C. Specimen B has pressure 18.20 MPa at 31 °C.</p>',
    review: ['Table cells must remain associated with the correct specimen.', 'Preserve σ, E, ε and multiplication without changing meaning.', 'Preserve 210 GPa, 0.002 and 420 MPa.', 'Identify Figure 1 and do not invent experimental evidence.'],
    criticalTokens: ['42.75', '18.20', '210', '0.002', '420', 'MPa', 'Figure'],
  },
];

app.whenReady().then(async () => {
  await mkdir(output, { recursive: true });
  const window = new BrowserWindow({ width: 1000, height: 1100, show: false, webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false } });
  async function load(html) {
    await window.loadURL(`data:text/html;charset=utf-8,${encodeURIComponent(`<!doctype html><meta charset="utf-8">${style}${html}`)}`);
    await window.webContents.executeJavaScript('document.fonts.ready');
  }
  async function pdf(name) {
    const bytes = await window.webContents.printToPDF({ printBackground: true, pageSize: 'A4' });
    await writeFile(path.join(output, `${name}.pdf`), bytes);
  }
  try {
    for (const fixture of fixtures) {
      await load(fixture.html); await pdf(fixture.name);
      await writeFile(path.join(output, `${fixture.name}.html`), `<!doctype html><meta charset="utf-8">${style}${fixture.html}`);
    }
    await load('<h1>Scanned Turkish / English fixture</h1><p>İstanbul — ölçülen basınç: 42,75 MPa.</p><p>Şişe sıcaklığı: 23 °C.</p><p>Measured pressure: 42.75 MPa.</p><p>Specimen temperature: 23 °C.</p><p>Public generated content for OCR review.</p>');
    const image = await window.webContents.executeJavaScript(`(() => {
      const canvas = document.createElement('canvas'); canvas.width = 1000; canvas.height = 650;
      const context = canvas.getContext('2d'); context.fillStyle = 'white'; context.fillRect(0, 0, 1000, 650);
      context.fillStyle = '#111'; context.font = '30px "DejaVu Sans", sans-serif';
      ['Scanned Turkish / English fixture', 'İstanbul — ölçülen basınç: 42,75 MPa.', 'Şişe sıcaklığı: 23 °C.', 'Measured pressure: 42.75 MPa.', 'Specimen temperature: 23 °C.', 'Public generated content for OCR review.'].forEach((line,i) => context.fillText(line, 35, 60 + i * 70));
      return canvas.toDataURL();
    })()`);
    await load(`<style>body{padding:0;margin:0}.scan{height:800px;display:flex;align-items:center;justify-content:center}.scan img{width:85%;transform:rotate(90deg)}</style><div class="scan"><img src="${image}"></div>`);
    await pdf('scanned-rotated-bilingual');
    const manifest = { generated: new Date().toISOString(), publicFixtures: true, cases: [...fixtures.map(({ name, review, criticalTokens, expectedWords }) => ({ name, pdf: `${name}.pdf`, review, criticalTokens, expectedWords })), { name: 'scanned-rotated-bilingual', pdf: 'scanned-rotated-bilingual.pdf', review: ['Recover rotated raster text without relying on native PDF text.', 'Preserve Turkish diacritics, pressure values, and temperature.'], criticalTokens: ['42.75', 'MPa', '23'], expectedWords: ['İstanbul', 'Şişe'] }], limits: ['Token presence is a regression signal, not human reading-order/citation acceptance.', 'These generated fixtures do not represent all scans, equations, or malformed PDFs.'] };
    await writeFile(path.join(output, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
    console.log(JSON.stringify({ status: 'ok', output, fixtures: manifest.cases.length }));
  } finally { window.destroy(); app.quit(); }
}).catch(error => { console.error(error.stack); app.exit(1); });
