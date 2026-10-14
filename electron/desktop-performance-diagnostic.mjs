// Runs the actual renderer with public fixtures and disposable test storage.
import { app, BrowserWindow } from 'electron';
import { writeFileSync, mkdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import os from 'node:os';
import { diagnosticEnvironment } from './diagnostic-environment.mjs';

async function run() {
  const root = resolve(import.meta.dirname, '..');
  const output = resolve(process.env.AYLA_PERFORMANCE_OUTPUT || `${root}/release/evidence/desktop-performance.json`);
  const sampleSeconds = Number(process.env.AYLA_PERFORMANCE_SECONDS || 10);
  if (!Number.isFinite(sampleSeconds) || sampleSeconds < 5 || sampleSeconds > 120) throw new Error('Sample duration must be 5–120 seconds');
  const env = diagnosticEnvironment();
  // A missing explicit config prevents checkout secrets/model choices entering the fixture.
  Object.assign(process.env, env, { AYLA_CONFIG_FILE: `${env.AYLA_DATA_DIR}/no-settings.env` });
  const database = env.SURREALDB_DB;
  const sqlHeaders = { Accept: 'application/json', Authorization: `Basic ${Buffer.from(`${env.SURREALDB_USER}:${env.SURREALDB_PASS}`).toString('base64')}` };
  async function sql(statement) {
    const response = await fetch(`${env.SURREALDB_URL}/sql`, { method: 'POST', headers: sqlHeaders, body: statement, signal: AbortSignal.timeout(30000) });
    const results = await response.json();
    if (!response.ok || !Array.isArray(results) || results.some(r => r.status !== 'OK')) throw new Error('Isolated graph fixture SQL failed');
  }
  const timestamp = new Date().toISOString();
  const nodes = Array.from({ length: 1000 }, (_, i) => ({ uid: `fixture_${i}`, name: `Public fixture ${i}`, label: 'Concept', node_type: 'Concept', sources: [`fixture_${Math.floor(i / 50)}`], origins: ['diagnostic'], confidence: 0.9, created: timestamp, updated: timestamp }));
  const edges = nodes.flatMap((n, i) => [1, 7].map(step => ({ uid: `edge_${i}_${step}`, from_uid: n.uid, to_uid: nodes[(i + step) % nodes.length].uid, relation: 'related_to', sources: n.sources, confidence: 0.9, created: timestamp, updated: timestamp })));
  await sql(`USE NS ${env.SURREALDB_NS} DB ${database}; INSERT INTO graph_node_v2 ${JSON.stringify(nodes)} RETURN NONE; INSERT INTO graph_edge_v2 ${JSON.stringify(edges)} RETURN NONE;`);
  await import('./main.mjs');
  await app.whenReady();
  const delay = ms => new Promise(r => setTimeout(r, ms));
  try {
    let window;
    let initialized = false;
    for (let i = 0; i < 120; i++) {
      window = BrowserWindow.getAllWindows()[0];
      if (window && !window.webContents.isLoading()) {
        const ready = await window.webContents.executeJavaScript('Boolean(window.__app && document.querySelector("#chat-input") && document.querySelector(".kb-tab"))').catch(() => false);
        if (ready) { initialized = true; break; }
      }
      await delay(500);
    }
    if (!initialized) throw new Error('Desktop did not initialize within 60 seconds');
    window.setContentSize(3440, 1440);
    window.setFullScreen(true);
    window.show();
    window.focus();
    await delay(3000);
    const results = [];
    for (const scenario of ['idle', 'synthetic-text-updates', 'knowledge-graph', 'relationship-gravity']) {
      console.error(`Sampling desktop scenario: ${scenario}`);
      const result = await window.webContents.executeJavaScript(`(async () => {
        const scenario = ${JSON.stringify(scenario)};
        const panels = [...document.querySelectorAll('.panel')];
        const displays = panels.map(p => p.style.display);
        panels.forEach(p => p.style.display = 'none');
        let fixture = null, timer = null;
        if (scenario === 'synthetic-text-updates') {
          fixture = document.createElement('div'); fixture.className = 'panel chat-panel';
          fixture.style.cssText = 'left:600px;top:150px;width:700px;height:550px';
          fixture.innerHTML = '<div class="panel-body"><div class="msg assistant"><div class="msg-content"></div></div></div>';
          document.body.appendChild(fixture);
          const content = fixture.querySelector('.msg-content'); let count = 0;
          timer = setInterval(() => { content.textContent += ' Public streaming fixture ' + (++count); }, 30);
        }
        if (scenario === 'knowledge-graph' || scenario === 'relationship-gravity') {
          const kb = document.querySelector('[data-panel-id="kb"]'); kb.style.display = '';
          kb.querySelector('[data-tab="graph"]').click();
          if (scenario === 'relationship-gravity') {
            const select = kb.querySelector('#kb-graph-view'); select.value = 'gravity'; select.dispatchEvent(new Event('change'));
            const deadline = performance.now() + 30000;
            while (document.querySelector('#kb-gravity-status')?.textContent?.includes('Calculating') && performance.now() < deadline) await new Promise(r => setTimeout(r, 100));
            if (/failed|unavailable|Calculating/i.test(document.querySelector('#kb-gravity-status')?.textContent || '')) throw new Error('Relationship Gravity did not finish successfully');
          }
        }
        const frameTimes = []; let previous = null;
        window.__app.diagnosticFrameTimes = [];
        const started = performance.now();
        await new Promise((resolve, reject) => { const deadline = setTimeout(() => reject(new Error('Animation sampling stalled; ensure the diagnostic window is visible')), ${sampleSeconds * 1000 + 5000}); function frame(now) { if (previous !== null) frameTimes.push(now - previous); previous = now; if (now - started >= ${sampleSeconds * 1000}) { clearTimeout(deadline); resolve(); } else requestAnimationFrame(frame); } requestAnimationFrame(frame); });
        clearInterval(timer); fixture?.remove();
        const metadata = { visibility: document.visibilityState, width: innerWidth, height: innerHeight, devicePixelRatio, avatar: window.__app?.avatarAvailable, renderQuality: window.__app?.renderQuality, renderResolution: window.__app?.renderResolution, graphStatus: scenario.includes('graph') || scenario === 'relationship-gravity' ? document.querySelector('#kb-gravity-status')?.textContent : null, graphRows: scenario.includes('graph') || scenario === 'relationship-gravity' ? document.querySelector('#kb-graph-count')?.textContent : null };
        panels.forEach((p,i) => p.style.display = displays[i]);
        const elapsed = performance.now() - started; frameTimes.sort((a,b) => a-b);
        const timestamps = window.__app.diagnosticFrameTimes; window.__app.diagnosticFrameTimes = null;
        const rendererIntervals = timestamps.slice(1).map((t,i) => t - timestamps[i]).sort((a,b) => a-b);
        const renderer = { frames: timestamps.length, fps: timestamps.length * 1000 / elapsed, p50FrameMs: rendererIntervals[Math.floor(rendererIntervals.length * .5)], p95FrameMs: rendererIntervals[Math.floor(rendererIntervals.length * .95)], maxFrameMs: rendererIntervals.at(-1) };
        return { scenario, ...metadata, renderer, frames: frameTimes.length, durationMs: elapsed, fps: frameTimes.length * 1000 / elapsed, p50FrameMs: frameTimes[Math.floor(frameTimes.length * .5)], p95FrameMs: frameTimes[Math.floor(frameTimes.length * .95)], maxFrameMs: frameTimes.at(-1) };
      })()`);
      results.push({ ...result, processes: app.getAppMetrics().map(({ type, cpu, memory }) => ({ type, cpu, memory })) });
    }
    const capabilities = await window.webContents.executeJavaScript('window.aylaDesktop.invoke("get_runtime_capabilities", {})');
    const report = { timestamp: new Date().toISOString(), platform: process.platform, kernel: os.release(), cpu: os.cpus()[0]?.model, totalMemoryBytes: os.totalmem(), session: process.env.XDG_SESSION_TYPE || 'unknown', requestedOzonePlatform: app.commandLine.getSwitchValue('ozone-platform') || 'default', electron: process.versions.electron, chrome: process.versions.chrome, gpu: await app.getGPUInfo('complete'), gpuFeatures: app.getGPUFeatureStatus(), storageReady: capabilities.storageReady, fixture: { nodes: nodes.length, edges: edges.length }, results, limitations: ['Synthetic text updates measure DOM pressure, not real model streaming.', 'requestAnimationFrame timing measures renderer scheduling, not compositor presentation.', 'One local session does not qualify other GPUs or X11/Wayland sessions.'] };
    mkdirSync(dirname(output), { recursive: true }); writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
    console.log(JSON.stringify({ output, results: results.map(({ processes, ...r }) => r) }, null, 2));
  } catch (error) {
    console.error(error.stack); process.exitCode = 1;
  } finally {
    await sql(`USE NS ${env.SURREALDB_NS}; REMOVE DATABASE ${database};`).catch(() => {});
    app.quit();
  }

}

run().catch(error => { console.error(error.stack); app.exit(1); });
