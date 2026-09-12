import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';

const root = path.resolve(import.meta.dirname, '..');
const read = relativePath => readFileSync(path.join(root, relativePath), 'utf8');

test('OCR and research are linked Rust libraries rather than helper processes', () => {
  const workspace = read('Cargo.toml');
  const coreManifest = read('crates/ayla-core/Cargo.toml');
  const coreApi = read('crates/ayla-core/src/api.rs');
  const research = read('crates/ayla-core/src/research.rs');
  const staging = read('electron/stage-resources.mjs');

  assert.match(workspace, /crates\/ayla-ocr/);
  assert.match(workspace, /crates\/ayla-research/);
  assert.match(coreManifest, /ayla-ocr\s*=\s*\{\s*path/);
  assert.match(coreManifest, /ayla-research\s*=\s*\{\s*path/);
  assert.match(coreApi, /ayla_ocr::extract_pdf/);
  assert.match(coreApi, /persist_ocr_artifacts/);
  assert.match(research, /ayla_research::search_with_progress/);
  assert.doesNotMatch(`${coreApi}\n${research}`, /Command::new|tokio::process/);
  assert.doesNotMatch(staging, /OCR helper|Meta Search helper|ocr-helper/);
});

test('GLM OCR stays in-process and uses PDFium without Poppler commands', () => {
  const ocrManifest = read('crates/ayla-ocr/Cargo.toml');
  const ocrLibrary = read('crates/ayla-ocr/src/lib.rs');
  const renderer = read('crates/ayla-ocr/src/pdf_renderer.rs');
  const client = read('crates/ayla-ocr/src/ocr_client.rs');
  const panel = read('src/ui/ocr-panel.js');

  assert.match(ocrManifest, /pdfium-bundled/);
  assert.match(ocrLibrary, /parallel_pages/);
  assert.match(renderer, /page\.text\(\)/);
  assert.match(client, /api\/generate/);
  assert.match(panel, /local GLM OCR with PDFium layout detection/);
  assert.match(ocrLibrary, /knowledge::build_knowledge_package/);
  assert.match(panel, /KB-ready document/);
  assert.match(panel, /ingest_ocr_knowledge/);
  assert.doesNotMatch(panel, /invoke\('approve_ocr_graph'/);
  assert.doesNotMatch(`${ocrLibrary}\n${renderer}`, /pdftoppm|pdftotext|Command::new/);
});

test('integrated module manifests never depend on sibling project paths', () => {
  const manifests = [
    'Cargo.toml',
    'crates/ayla-core/Cargo.toml',
    'crates/ayla-stt/Cargo.toml',
    'crates/ayla-tts/Cargo.toml',
    'crates/ayla-ocr/Cargo.toml',
    'crates/ayla-code-analysis/Cargo.toml',
  ].map(read).join('\n');
  assert.doesNotMatch(
    manifests,
    /\/home\/mka\/Code\/(?:ayla_stt_test_module|ayla_tts_test_module|glm_ocr_tests|kb_ready_code_doc)/,
  );
  assert.match(manifests, /ayla-code-analysis\s*=\s*\{\s*path\s*=\s*"\.\.\/ayla-code-analysis"/);
});

test('TTS language selection is validated in Rust and routed from the Status panel', () => {
  const engine = read('crates/ayla-tts/src/lib.rs');
  const service = read('crates/ayla-core/src/tts.rs');
  const sidecar = read('crates/ayla-sidecar/src/dispatcher.rs');
  const main = read('src/main.js');
  const status = read('src/ui/status-panel.js');

  assert.match(engine, /enum TtsLanguage/);
  assert.match(engine, /Value::String\(language\.code\(\)\.into\(\)\)/);
  assert.match(service, /generate_with_language/);
  assert.match(service, /supported_languages/);
  assert.match(sidecar, /params\.language\.as_deref\(\)/);
  assert.match(main, /language: request\.language \|\| state\.ttsLanguage/);
  assert.match(status, /TTS language/);
  assert.match(status, /\['tr', 'Türkçe'\]/);
  assert.match(status, /\['en', 'English'\]/);
});

test('STT supports Turkish and English with independent system and Chat controls', () => {
  const service = read('crates/ayla-core/src/stt.rs');
  const commands = read('crates/ayla-core/src/commands.rs');
  const sidecar = read('crates/ayla-sidecar/src/dispatcher.rs');
  const capture = read('src/stt-capture.js');
  const chat = read('src/ui/chat-panel.js');
  const status = read('src/ui/status-panel.js');

  assert.match(service, /language_mode: "auto"/);
  assert.match(service, /vec!\["tr"\.into\(\), "en"\.into\(\)\]/);
  assert.match(service, /set_system_enabled/);
  assert.match(commands, /set_system_stt_enabled/);
  assert.match(sidecar, /set_system_stt_enabled/);
  assert.match(capture, /this\.deferred && this\.recordedBytes/);
  assert.match(chat, /deferred: true/);
  assert.match(chat, /Press the microphone again to stop and transcribe/);
  assert.match(chat, /Recognized:/);
  assert.doesNotMatch(chat, /systemSttEnabled/);
  assert.match(status, /System STT: OPEN/);
  assert.match(status, /System STT: CLOSED/);
  assert.match(status, /manual Chat microphone remains available/i);
});

test('code analysis stays in-process and graph-v2 commits through staging and backups', () => {
  const graphV2 = read('crates/ayla-core/src/memory/graph_v2.rs');
  const coreApi = read('crates/ayla-core/src/api.rs');
  const config = read('crates/ayla-core/src/config.rs');
  const analyzerPanel = read('src/ui/code-analysis-panel.js');
  const ocrPanel = read('src/ui/ocr-panel.js');
  const productionAdapter = graphV2.split('#[cfg(test)]')[0];
  assert.match(graphV2, /from_code_analysis/);
  assert.match(graphV2, /missing target/);
  assert.match(graphV2, /remove_source/);
  assert.match(graphV2, /BEGIN TRANSACTION/);
  assert.match(graphV2, /graph_node_v2_backup_/);
  assert.match(coreApi, /generate_deterministic/);
  assert.match(coreApi, /persist_code_analysis_artifacts/);
  assert.match(coreApi, /get_code_analysis_runs/);
  assert.match(coreApi, /persist_code_analysis_diff/);
  assert.match(coreApi, /index_run\(operation_id, &documents\)/);
  assert.match(coreApi, /delete_code_analysis_run/);
  assert.match(coreApi, /load_code_analysis_private_source/);
  assert.match(coreApi, /source_snapshot_stored/);
  assert.match(coreApi, /config\.code_analysis_model/);
  assert.match(config, /AYLA_CODE_ANALYSIS_MODEL/);
  assert.match(config, /AYLA_CODE_ANALYSIS_ALLOW_CLOUD/);
  assert.match(coreApi, /code_analysis_allow_cloud/);
  assert.match(coreApi, /code_semantic_cloud_disabled/);
  assert.match(config, /&\["AYLA_OCR_ASSEMBLY_MODEL"\]/);
  assert.doesNotMatch(config, /&\["AYLA_OCR_ASSEMBLY_MODEL", "AYLA_LLM_MODEL"\]/);
  assert.match(config, /AYLA_OCR_ASSEMBLY_ALLOW_CLOUD/);
  assert.match(ocrPanel, /Cloud models receive bounded OCR page evidence/);
  assert.match(analyzerPanel, /Approve import/);
  assert.match(analyzerPanel, /Re-analyze and diff/);
  assert.match(analyzerPanel, /Ollama semantic \(cloud enabled\)/);
  assert.match(analyzerPanel, /may send selected excerpts to the configured cloud model/);
  assert.match(analyzerPanel, /events\.emit\('kb-refresh'\)/);
  assert.match(analyzerPanel, /Knowledge Base → Structured or Graph v2/);
  assert.match(analyzerPanel, /private snippet snapshot saved/);
  const artifacts = read('crates/ayla-core/src/artifacts.rs');
  assert.match(artifacts, /persist_code_analysis_private_source/);
  assert.match(artifacts, /source\.snippet/);
  assert.match(artifacts, /from_mode\(0o600\)/);
  assert.doesNotMatch(artifacts, /\("code-analysis", "source"\)/);
  assert.doesNotMatch(coreApi, /Command::new|tokio::process/);
  assert.doesNotMatch(productionAdapter, /source_excerpt/);
});

test('knowledge base reads Graph-v2 with bounded history and recoverable rollback', () => {
  const graphV2 = read('crates/ayla-core/src/memory/graph_v2.rs');
  const coreApi = read('crates/ayla-core/src/api.rs');
  const kbPanel = read('src/ui/kb-panel.js');
  const sidecar = read('crates/ayla-sidecar/src/dispatcher.rs');

  assert.match(graphV2, /APPLICATION_NODE_LIMIT/);
  assert.match(graphV2, /GraphImportRunV2/);
  assert.match(graphV2, /action: "remove"/);
  assert.match(graphV2, /action: "rollback"/);
  assert.match(graphV2, /replace_legacy/);
  assert.match(graphV2, /prune_backups/);
  assert.match(coreApi, /get_graph_v2_data/);
  assert.match(coreApi, /migrate_legacy_graph_v2/);
  assert.match(sidecar, /rollback_graph_v2/);
  assert.match(kbPanel, /invoke\('get_graph_v2_data'\)/);
  assert.match(kbPanel, /get_graph_v2_page/);
  assert.match(kbPanel, /Graph v2 \(UID\)/);
  assert.match(kbPanel, /All origins/);
  assert.match(kbPanel, /Restore before/);
});
