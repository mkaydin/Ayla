#!/usr/bin/env bash
# ───────────────────────────────────────────────────────────
# Ayla System Doctor — comprehensive system health check
# Usage: bash doctor.sh [--quick] [--no-docker] [--no-ollama]
# ───────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PASS=0; FAIL=0; SKIP=0; TOTAL=0
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'
CHECK_MARK="✓"; CROSS_MARK="✗"; SKIP_MARK="○"

# Flags
QUICK=false; NO_DOCKER=false; NO_OLLAMA=false
START_TIME=$(date +%s)

for arg in "$@"; do
  case "$arg" in
    --quick) QUICK=true ;;
    --no-docker) NO_DOCKER=true ;;
    --no-ollama) NO_OLLAMA=true ;;
  esac
done

log_section() { echo -e "\n${BOLD}${CYAN}━━━ $1 ━━━${NC}"; }
# NOTE: `((x++))` returns exit status 1 when x evaluates to 0 (i.e. the FIRST increment of a
# counter starting at 0). Under `set -e` that terminates the whole script. Append `|| true`
# so the counters never trip the shell (issue I6).
log_pass()   { ((PASS++))  || true; ((TOTAL++)) || true; echo -e "  ${GREEN}${CHECK_MARK}${NC} $1"; }
log_fail()   { ((FAIL++))  || true; ((TOTAL++)) || true; echo -e "  ${RED}${CROSS_MARK}${NC} $1 — $2"; }
log_skip()   { ((SKIP++))  || true; ((TOTAL++)) || true; echo -e "  ${YELLOW}${SKIP_MARK}${NC} $1 (skipped)"; }
log_info()   { echo -e "  ${CYAN}→${NC} $1"; }

check_cmd() { command -v "$1" >/dev/null 2>&1; }

# ───────────────────────────────────────────────────────────
# SECTION 1 — Environment & Prerequisites
# ───────────────────────────────────────────────────────────
log_section "1. Environment & Prerequisites"

check_cmd rustc && log_pass "Rust $(rustc --version | cut -d' ' -f2)" \
  || log_fail "Rust not found" "Install: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"

check_cmd node   && log_pass "Node $(node --version)" \
  || log_fail "Node not found" "Install Node.js 18+"

check_cmd npm    && log_pass "npm $(npm --version 2>/dev/null || echo '?')" \
  || log_fail "npm not found" "Install Node.js"

check_cmd cargo  && log_pass "Cargo $(cargo --version | cut -d' ' -f2)" \
  || log_fail "Cargo not found" "Install Rust"

check_cmd jq     && log_pass "jq $(jq --version)" \
  || { log_info "jq not found — some JSON checks will be skipped"; }

check_cmd curl   && log_pass "curl available" \
  || log_fail "curl not found" "Install curl"

if [ -f "$SCRIPT_DIR/package.json" ]; then
  log_pass "package.json found"
else
  log_fail "package.json missing" "Run from project root"
fi

if [ -f "$SCRIPT_DIR/docker-compose.yml" ]; then
  log_pass "docker-compose.yml found"
else
  log_fail "docker-compose.yml missing" "File was deleted"
fi

# Count top-level deps robustly (issue I12). Prefer jq; fall back to counting
# package.json `dependencies` keys directly (node is required for this project anyway).
NODE_DEPS=$(cd "$SCRIPT_DIR" && {
  npm ls --depth=0 --json 2>/dev/null | jq '.dependencies | length' 2>/dev/null \
    || node -e 'console.log(Object.keys(require("./package.json").dependencies||{}).length)' 2>/dev/null \
    || echo 0
})
[ "$NODE_DEPS" -gt 0 ] 2>/dev/null && log_pass "Node dependencies installed ($NODE_DEPS packages)" \
  || log_fail "Node dependencies missing" "Run: npm install"

# ───────────────────────────────────────────────────────────
# SECTION 2 — Docker Services
# ───────────────────────────────────────────────────────────
log_section "2. Docker Services"

if [ "$NO_DOCKER" = true ]; then
  log_skip "Docker checks disabled via --no-docker"
else
  check_cmd docker || { log_fail "Docker not found" "Install Docker"; }
  docker info >/dev/null 2>&1 && log_pass "Docker daemon running" \
    || log_fail "Docker daemon not running" "Start Docker"

  # Check each container
  declare -A CONTAINERS=(
    ["ayla-surrealdb"]="SurrealDB"
    ["ayla-qdrant"]="Qdrant"
    ["ayla-searxng"]="SearXNG"
    ["ayla-lightpanda"]="Lightpanda"
  )

  for cname in "${!CONTAINERS[@]}"; do
    label="${CONTAINERS[$cname]}"
    if docker ps --format '{{.Names}}' 2>/dev/null | grep -q "$cname"; then
      status=$(docker inspect --format='{{.State.Status}}' "$cname" 2>/dev/null || echo "unknown")
      health=$(docker inspect --format='{{.State.Health.Status}}' "$cname" 2>/dev/null || echo "none")
      if [ "$status" = "running" ] && [ "$health" = "healthy" ]; then
        log_pass "$label — running (healthy)"
      elif [ "$status" = "running" ]; then
        log_info "$label — running (starting...)"
        log_pass "$label — container running"
      else
        log_fail "$label — status: $status" "Check: docker compose ps"
      fi
    else
      log_info "$label — not running (start with: docker compose up -d)"
      log_skip "$label"
    fi
  done

  # Health endpoints (only if containers are running)
  echo ""
  log_info "Health endpoints:"

  if curl -sf --max-time 3 http://localhost:8000/health >/dev/null 2>&1; then
    log_pass "SurrealDB :8000 — responds"
  else
    log_fail "SurrealDB :8000 — unreachable" "Start: docker compose up -d surrealdb"
  fi

  if curl -sf --max-time 3 http://localhost:6333/health >/dev/null 2>&1; then
    log_pass "Qdrant :6333 — responds"
  else
    log_fail "Qdrant :6333 — unreachable" "Start: docker compose up -d qdrant"
  fi

  if curl -sf --max-time 5 "http://localhost:8888/search?q=test&format=json" >/dev/null 2>&1; then
    log_pass "SearXNG :8888 — responds"
  else
    log_skip "SearXNG :8888 — unreachable"
  fi

  if curl -sf --max-time 3 http://localhost:9222/json/version >/dev/null 2>&1; then
    log_pass "Lightpanda :9222 — responds"
  else
    log_skip "Lightpanda :9222 — unreachable"
  fi
fi

# ───────────────────────────────────────────────────────────
# SECTION 3 — Ollama
# ───────────────────────────────────────────────────────────
log_section "3. Ollama"

if [ "$NO_OLLAMA" = true ]; then
  log_skip "Ollama checks disabled via --no-ollama"
else
  check_cmd ollama || { log_fail "Ollama CLI not found" "Install: curl -fsSL https://ollama.com/install.sh | sh"; }

  # Ollama server
  if curl -sf --max-time 3 http://localhost:11434/api/tags >/dev/null 2>&1; then
    log_pass "Ollama server :11434 — running"
    MODEL_COUNT=$(curl -sf http://localhost:11434/api/tags 2>/dev/null | jq '.models | length' 2>/dev/null || echo 0)
    log_info "Models available: $MODEL_COUNT"
  else
    log_fail "Ollama server :11434 — not running" "Start: ollama serve"
  fi

  # Required models
  for model in "gemma4:e4b" "qwen3-embedding:0.6b"; do
    if ollama list 2>/dev/null | grep -q "$model"; then
      size=$(ollama list 2>/dev/null | grep "$model" | awk '{print $3, $4}')
      log_pass "$model — pulled ($size)"
    else
      log_fail "$model — not pulled" "Run: ollama pull $model"
    fi
  done

  # Optional: deepseek-ocr
  if ollama list 2>/dev/null | grep -q "deepseek-ocr"; then
    log_pass "deepseek-ocr — pulled (optional)"
  else
    log_skip "deepseek-ocr (optional — needed for PDF OCR)"
  fi

  # Embedding API test
  if [ "$QUICK" = false ]; then
    log_info "Testing embedding API..."
    EMBED_LEN=$(curl -sf --max-time 10 http://localhost:11434/api/embed \
      -d '{"model":"qwen3-embedding:0.6b","input":["hello world"]}' \
      2>/dev/null | jq '.embeddings[0] | length' 2>/dev/null || echo 0)
    if [ "$EMBED_LEN" -eq 1024 ]; then
      log_pass "Embedding API — 1024-dim vector returned"
    elif [ "$EMBED_LEN" -gt 0 ]; then
      log_info "Embedding API — returned ${EMBED_LEN}-dim vector (expected 1024)"
      log_pass "Embedding API — responds"
    else
      log_fail "Embedding API — failed" "Check: ollama pull qwen3-embedding:0.6b"
    fi
  else
    log_skip "Embedding API test (--quick mode)"
  fi

  # Chat API test
  if [ "$QUICK" = false ]; then
    log_info "Testing chat API (gemma4:e4b)..."
    CHAT_RESP=$(curl -sf --max-time 30 http://localhost:11434/api/chat \
      -d '{"model":"gemma4:e4b","messages":[{"role":"user","content":"Say OK"}],"stream":false,"options":{"num_predict":10}}' \
      2>/dev/null | jq -r '.message.content' 2>/dev/null || echo "")
    if [ -n "$CHAT_RESP" ]; then
      log_pass "Chat API — responds: '${CHAT_RESP:0:60}'"
    else
      log_fail "Chat API — failed" "Check: ollama pull gemma4:e4b"
    fi
  else
    log_skip "Chat API test (--quick mode)"
  fi
fi

# ───────────────────────────────────────────────────────────
# SECTION 4 — Rust Compilation
# ───────────────────────────────────────────────────────────
log_section "4. Rust Backend"

cd "$SCRIPT_DIR/src-tauri"

log_info "Running: cargo check (this may take some time)..."
# Capture output ONCE and reuse it (issue I20 — was: cargo check ran twice, recompiling from scratch).
CARGO_OUT="$(cargo check 2>&1 || true)"
if printf '%s\n' "$CARGO_OUT" | grep -qE '^error' ; then
  log_fail "cargo check — compilation errors" "Fix errors shown above"
  printf '%s\n' "$CARGO_OUT" | grep -E '^error' | head -20
else
  log_pass "cargo check — no errors"
fi

# Check for warnings (from the same captured output).
WARN_COUNT="$(printf '%s\n' "$CARGO_OUT" | grep -c 'warning:' || true)"
if [ "$WARN_COUNT" -gt 0 ]; then
  log_info "$WARN_COUNT compiler warnings (non-blocking)"
fi

# Shared backend module presence. Business logic moved to ayla-core during the
# Electron migration; src-tauri now contains only the thin host adapters.
for mod in "memory/mod.rs" "config.rs" \
           "memory/conversation.rs" "memory/semantic.rs" "memory/graph.rs" \
           "memory/episodic.rs" "memory/embeddings.rs" "memory/extraction.rs" \
           "memory/summarizer.rs" "memory/consolidator.rs" "memory/retrieval.rs" \
           "memory/context.rs" "memory/pipeline.rs" "memory/ingestion.rs" \
           "memory/working.rs" "memory/document_store.rs" \
           "db/mod.rs" "db/surreal.rs" "db/qdrant.rs" \
           "tools/mod.rs" "tools/types.rs" "tools/ocr.rs" "tools/meta_search.rs" \
           "tools/quick_extract.rs"; do
  if [ -f "$SCRIPT_DIR/crates/ayla-core/src/$mod" ]; then
    : # silent pass
  else
    log_fail "Missing ayla-core module: $mod" "File was deleted"
  fi
done
[ -f "$SCRIPT_DIR/src-tauri/src/memory/commands.rs" ] \
  || log_fail "Missing Tauri command adapter" "Check src-tauri/src/memory/commands.rs"

# Check Tauri commands registered in lib.rs (issue I18: hardcoded count drifted to 11
# while only 10 were actually checked). Now derive the count from lib.rs itself.
TAURI_CMD_COUNT="$(grep -cE 'tts::[a-z_]+|memory::commands::[a-z_]+' src/lib.rs || true)"
TAURI_CMDS_OK=true
for cmd in enrich_prompt process_turn process_message get_kb_data run_ocr run_search \
           ingest_text run_consolidation generate_tts stop_tts; do
  grep -q "$cmd" src/lib.rs || TAURI_CMDS_OK=false
done
if [ "$TAURI_CMDS_OK" = "true" ]; then
  log_pass "$TAURI_CMD_COUNT Tauri commands registered"
else
  log_fail "Tauri command registration incomplete" "Check src/lib.rs"
fi

[ -f "$SCRIPT_DIR/crates/ayla-ocr/src/lib.rs" ] \
  && log_pass "In-process OCR crate available" \
  || log_fail "Missing ayla-ocr crate" "Check crates/ayla-ocr"
[ -f "$SCRIPT_DIR/crates/ayla-research/src/lib.rs" ] \
  && log_pass "In-process research crate available" \
  || log_fail "Missing ayla-research crate" "Check crates/ayla-research"

cd "$SCRIPT_DIR"

# ───────────────────────────────────────────────────────────
# SECTION 5 — Frontend
# ───────────────────────────────────────────────────────────
log_section "5. Frontend"

if [ "$QUICK" = true ]; then
  log_skip "Frontend build (--quick mode)"
else
  log_info "Running: npm run build..."
  if npm run build 2>&1 | tail -3; then
    log_pass "npm run build — success"
  else
    log_fail "npm run build — failed" "Check error output above"
  fi
fi

# CSS files
for css in "tokens.css" "panel.css" "scrollbar.css" "status.css" \
           "chat.css" "ocr.css" "search.css" "kb.css" "sidebar.css"; do
  [ -f "$SCRIPT_DIR/styles/$css" ] || log_fail "Missing: styles/$css" "File was deleted"
done

# JS modules (issue I19: hardcoded "17" but 16 listed. Now compute the actual count.)
JS_MODULES=(
  "src/lib/ipc.js" "src/lib/markdown.js" "src/lib/drag-resize.js" \
  "src/lib/events.js" "src/lib/prompts.js" "src/lib/process-notify.js" \
  "src/lib/ayla-responses.js" \
  "src/ui/panel-system.js" "src/ui/status-panel.js" "src/ui/chat-panel.js" \
  "src/ui/ocr-panel.js" "src/ui/search-panel.js" "src/ui/kb-panel.js" \
  "src/ui/sidebar.js" \
  "src/main.js" "src/tts-player.js"
)
JS_OK=true
for js in "${JS_MODULES[@]}"; do
  [ -f "$SCRIPT_DIR/$js" ] || { log_fail "Missing: $js" "File was deleted"; JS_OK=false; }
done
[ "$JS_OK" = "true" ] && log_pass "All ${#JS_MODULES[@]} JS modules present"

# HTML
[ -f "$SCRIPT_DIR/index.html" ] && log_pass "index.html found" \
  || log_fail "index.html missing"

# Check Ayla prompt in prompts.js
if grep -q "AYLA.*AI coworker" "$SCRIPT_DIR/src/lib/prompts.js" 2>/dev/null; then
  log_pass "Ayla coworker identity prompt present"
else
  log_fail "Ayla identity prompt missing" "Check src/lib/prompts.js"
fi

# Package.json deps
for dep in "@tauri-apps/api" "marked" "pixi.js" "pixi-live2d-display" "@tauri-apps/plugin-shell"; do
  if grep -q "\"$dep\"" "$SCRIPT_DIR/package.json" 2>/dev/null; then
    : # silent
  else
    log_fail "Missing npm dep: $dep" "Check package.json"
  fi
done

# ───────────────────────────────────────────────────────────
# SECTION 6 — Integration Smoke Tests
# ───────────────────────────────────────────────────────────
log_section "6. Integration Smoke Tests"

# Config files
[ -f "$SCRIPT_DIR/vite.config.js" ] && log_pass "vite.config.js — Ollama proxy configured" \
  || log_fail "vite.config.js missing"

[ -f "$SCRIPT_DIR/src-tauri/tauri.conf.json" ] && log_pass "tauri.conf.json found" \
  || log_fail "tauri.conf.json missing"

[ -f "$SCRIPT_DIR/src-tauri/capabilities/default.json" ] && log_pass "Tauri capabilities configured" \
  || log_fail "capabilities/default.json missing"

# Cargo.toml deps (note: project talks to SurrealDB over HTTP via `reqwest`, so there is no
# `surrealdb` crate — don't check for it).
for crate in "qdrant-client" "petgraph" "tokio" "reqwest" \
             "uuid" "chrono" "async-trait" "tauri-plugin-shell"; do
  if grep -q "$crate" "$SCRIPT_DIR/src-tauri/Cargo.toml" 2>/dev/null; then
    : # silent
  else
    log_fail "Missing Rust dep: $crate" "Check src-tauri/Cargo.toml"
  fi
done

# SearXNG config
[ -f "$SCRIPT_DIR/searxng/settings.yml" ] && log_pass "SearXNG settings.yml present" \
  || log_fail "searxng/settings.yml missing"
[ -f "$SCRIPT_DIR/searxng/limiter.toml" ] && log_pass "SearXNG limiter.toml present" \
  || log_fail "searxng/limiter.toml missing"

# .gitignore
for pattern in "data/" "node_modules" "dist/" "src-tauri/target"; do
  grep -q "$pattern" "$SCRIPT_DIR/.gitignore" 2>/dev/null \
    || log_fail ".gitignore missing: $pattern" "Add to .gitignore"
done

# Licensed Live2D model files are installed locally and intentionally gitignored.
LIVE2D_MODEL_DIR="${AYLA_LIVE2D_MODEL_DIR:-$SCRIPT_DIR/Mk6_1.0}"
for model_file in "Mk6.model3.json" "Mk6.moc3" "Mk6.physics3.json"; do
  if [ -f "$LIVE2D_MODEL_DIR/$model_file" ]; then
    : # silent
  else
    log_fail "Missing Live2D: $model_file" "Install Mk6_1.0 or set AYLA_LIVE2D_MODEL_DIR"
  fi
done

# Authored expressions and motions
EXPR_COUNT=$(find "$LIVE2D_MODEL_DIR/Expressions" -maxdepth 1 -type f -name '*.exp3.json' 2>/dev/null | wc -l)
MOTION_COUNT=$(find "$LIVE2D_MODEL_DIR/Motions" -maxdepth 1 -type f -name '*.motion3.json' 2>/dev/null | wc -l)
[ "$EXPR_COUNT" -ge 21 ] && log_pass "Live2D expressions: $EXPR_COUNT/21" \
  || log_fail "Live2D expressions: $EXPR_COUNT/21" "Missing Mk6 expression files"
[ "$MOTION_COUNT" -ge 3 ] && log_pass "Live2D motions: $MOTION_COUNT/3" \
  || log_fail "Live2D motions: $MOTION_COUNT/3" "Missing Mk6 motion files"

# TTS model
if [ -f "$SCRIPT_DIR/tts/tts_piper_amy_low/en_US-amy-low.onnx" ]; then
  log_pass "TTS model (amy-low) present"
else
  log_info "TTS model not found — download: see README.md Model Files section"
  log_skip "TTS model (amy-low)"
fi

# Sherpa-onnx vendored lib
if [ -d "$SCRIPT_DIR/tts/sherpa-onnx" ]; then
  log_pass "Sherpa-ONNX vendored lib present"
else
  log_skip "Sherpa-ONNX vendored (may need git submodule init)"
fi

# ───────────────────────────────────────────────────────────
# SECTION 7 — Database Integration (if SurrealDB is up)
# ───────────────────────────────────────────────────────────
if [ "$QUICK" = false ] && [ "$NO_DOCKER" = false ]; then
  log_section "7. Database Integration"

  if curl -sf --max-time 3 http://localhost:8000/health >/dev/null 2>&1; then
    # Test SurrealDB query
    RESULT=$(curl -sf --max-time 5 -X POST http://localhost:8000/sql \
      -H "Content-Type: application/json" \
      -H "Accept: application/json" \
      -u "root:root" \
      -d '{"ns":"ayla","db":"memory","sql":"SELECT * FROM fact LIMIT 1"}' 2>/dev/null || echo "")
    if [ -n "$RESULT" ]; then
      log_pass "SurrealDB query — responded"
    else
      log_info "SurrealDB query — no response (may need init)"
    fi

    # Check tables exist
    TABLES=$(curl -sf --max-time 5 -X POST http://localhost:8000/sql \
      -H "Content-Type: application/json" \
      -H "Accept: application/json" \
      -u "root:root" \
      -d '{"ns":"ayla","db":"memory","sql":"INFO FOR DB;"}' 2>/dev/null | jq -r '.[0].result.tables | keys | join(", ")' 2>/dev/null || echo "")
    if [ -n "$TABLES" ]; then
      log_info "SurrealDB tables: $TABLES"
    fi
  else
    log_skip "Database integration tests (SurrealDB not running)"
  fi

  # Qdrant
  if curl -sf --max-time 3 http://localhost:6333/health >/dev/null 2>&1; then
    COLLECTIONS=$(curl -sf --max-time 5 http://localhost:6333/collections 2>/dev/null | jq -r '.result.collections[].name' 2>/dev/null | tr '\n' ', ' || echo "")
    if [ -n "$COLLECTIONS" ]; then
      log_info "Qdrant collections: $COLLECTIONS"
    fi
    log_pass "Qdrant — accessible"
  else
    log_skip "Qdrant integration tests (not running)"
  fi
fi

# ───────────────────────────────────────────────────────────
# SECTION 8 — Code Quality
# ───────────────────────────────────────────────────────────
log_section "8. Code Quality"

# Check no leftover debug statements in JS
# NOTE: under `set -o pipefail`, `grep | wc -l || echo 0` is a bug — when grep finds nothing
# it exits 1, so pipefail makes the pipeline exit 1, and `|| echo 0` appends a SECOND 0 to
# wc's own 0, yielding "0\n0" which breaks `[ ... -gt 0 ]`. wc -l always emits a number, so
# no fallback is needed.
DEBUG_COUNT=$(grep -r "console\.\(log\|debug\)" "$SCRIPT_DIR/src" --include="*.js" 2>/dev/null | grep -v "warn\|error" | wc -l | tr -d ' ' || true)
if [ "$DEBUG_COUNT" -le 5 ]; then
  log_pass "JS console.log/debug: $DEBUG_COUNT (acceptable)"
else
  log_info "JS console.log/debug: $DEBUG_COUNT (consider removing some)"
fi

# Check for TODO comments
TODO_COUNT=$(grep -r "TODO\|FIXME\|HACK" "$SCRIPT_DIR/src" "$SCRIPT_DIR/src-tauri/src" --include="*.js" --include="*.rs" 2>/dev/null | wc -l | tr -d ' ' || true)
if [ "$TODO_COUNT" -gt 0 ]; then
  log_info "TODOs/FIXMEs remaining: $TODO_COUNT"
fi

# Check for hardcoded secrets (issue I11: the previous detector's `grep -v` excluded exactly
# the tokens being hunted, so it could never fire. Look for concrete assignments instead.)
SECRET_HITS="$(grep -rnE '"(root|admin|password|passwd|secret|token|api_?key)"\s*[,)]' \
  "$SCRIPT_DIR/src-tauri/src" --include="*.rs" 2>/dev/null || true)"
if [ -n "$SECRET_HITS" ]; then
  log_info "Potential hardcoded credentials found — review:"
  printf '%s\n' "$SECRET_HITS" | head -10
fi

# Line counts
RS_LINES=$(find "$SCRIPT_DIR/src-tauri/src" -name "*.rs" -print0 2>/dev/null | xargs -0 wc -l 2>/dev/null | tail -1 | awk '{print $1}')
JS_LINES=$(find "$SCRIPT_DIR/src" -name "*.js" -print0 2>/dev/null | xargs -0 wc -l 2>/dev/null | tail -1 | awk '{print $1}')
CSS_LINES=$(find "$SCRIPT_DIR/styles" -name "*.css" -print0 2>/dev/null | xargs -0 wc -l 2>/dev/null | tail -1 | awk '{print $1}')
log_info "Code size: $RS_LINES Rust · $JS_LINES JS · $CSS_LINES CSS"

# ───────────────────────────────────────────────────────────
# SECTION 9 — Quick API Tests (only if not --quick)
# ───────────────────────────────────────────────────────────
if [ "$QUICK" = false ] && [ "$NO_OLLAMA" = false ]; then
  log_section "9. API Integration Tests"

  if curl -sf --max-time 3 http://localhost:11434/api/tags >/dev/null 2>&1; then
    # Tool dispatch test
    log_info "Testing LLM tool dispatch..."
    DISPATCH=$(curl -sf --max-time 15 http://localhost:11434/api/chat \
      -d '{"model":"gemma4:e4b","messages":[{"role":"user","content":"search papers on Rust"}],"stream":false,"options":{"temperature":0,"num_predict":64}}' \
      2>/dev/null | jq -r '.message.content' 2>/dev/null || echo "")
    if [ -n "$DISPATCH" ]; then
      log_pass "LLM dispatch — responds"
    else
      log_fail "LLM dispatch — failed" "Ollama may be busy"
    fi

    # Entity extraction test (quick)
    log_info "Testing entity extraction..."
    ENTITIES=$(curl -sf --max-time 20 http://localhost:11434/api/chat \
      -d '{"model":"gemma4:e4b","messages":[{"role":"user","content":"Extract entities from: the user works with Rust and Tauri.\nOutput ONLY a JSON array."}],"stream":false,"options":{"temperature":0,"num_predict":128}}' \
      2>/dev/null | jq -r '.message.content' 2>/dev/null || echo "")
    if echo "$ENTITIES" | grep -q "Rust"; then
      log_pass "Entity extraction — found 'Rust'"
    else
      log_info "Entity extraction — response: ${ENTITIES:0:80}"
    fi
  else
    log_skip "API tests (Ollama not running)"
  fi
else
  log_skip "API integration tests (--quick or --no-ollama)"
fi

# ───────────────────────────────────────────────────────────
# SUMMARY
# ───────────────────────────────────────────────────────────
ELAPSED=$(($(date +%s) - START_TIME))
echo ""
echo -e "${BOLD}══════════════════════════════════════════${NC}"
echo -e "${BOLD}  DOCTOR REPORT${NC}"
echo -e "${BOLD}══════════════════════════════════════════${NC}"
echo -e "  ${GREEN}PASS${NC}: $PASS  ${RED}FAIL${NC}: $FAIL  ${YELLOW}SKIP${NC}: $SKIP  TOTAL: $TOTAL"
echo -e "  Time: ${ELAPSED}s"

if [ "$FAIL" -eq 0 ]; then
  echo -e "\n  ${GREEN}${BOLD}✓ All checks passed. System is healthy.${NC}"
  echo -e "  Run with: ${CYAN}npm run tauri dev${NC}"
  EXIT_CODE=0
else
  echo -e "\n  ${RED}${BOLD}✗ $FAIL check(s) failed. Review the issues above.${NC}"
  EXIT_CODE=1
fi

if [ "$SKIP" -gt 0 ]; then
  echo -e "  ${YELLOW}$SKIP check(s) were skipped. Run without --quick for full coverage.${NC}"
fi

echo ""
exit $EXIT_CODE
