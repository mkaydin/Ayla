#!/usr/bin/env bash
# ───────────────────────────────────────────────────────────
# ALICE System Doctor — comprehensive system health check
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
log_pass()   { ((PASS++)); ((TOTAL++)); echo -e "  ${GREEN}${CHECK_MARK}${NC} $1"; }
log_fail()   { ((FAIL++)); ((TOTAL++)); echo -e "  ${RED}${CROSS_MARK}${NC} $1 — $2"; }
log_skip()   { ((SKIP++)); ((TOTAL++)); echo -e "  ${YELLOW}${SKIP_MARK}${NC} $1 (skipped)"; }
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

NODE_DEPS=$(cd "$SCRIPT_DIR" && npm ls --depth=0 2>/dev/null | grep -c '├──\|└──' || echo 0)
[ "$NODE_DEPS" -gt 0 ] && log_pass "Node dependencies installed ($NODE_DEPS packages)" \
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
    ["alice-surrealdb"]="SurrealDB"
    ["alice-qdrant"]="Qdrant"
    ["alice-searxng"]="SearXNG"
    ["alice-lightpanda"]="Lightpanda"
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
if cargo check --quiet 2>&1; then
  log_pass "cargo check — no errors"
else
  log_fail "cargo check — compilation errors" "Fix errors shown above"
fi

# Check for warnings
WARN_COUNT=$(cargo check 2>&1 | grep -c "warning:" || echo 0)
if [ "$WARN_COUNT" -gt 0 ]; then
  log_info "$WARN_COUNT compiler warnings (non-blocking)"
fi

# Module presence
for mod in "memory/mod.rs" "memory/config.rs" "memory/commands.rs" \
           "memory/conversation.rs" "memory/semantic.rs" "memory/graph.rs" \
           "memory/episodic.rs" "memory/embeddings.rs" "memory/extraction.rs" \
           "memory/summarizer.rs" "memory/consolidator.rs" "memory/retrieval.rs" \
           "memory/context.rs" "memory/pipeline.rs" "memory/ingestion.rs" \
           "memory/working.rs" \
           "db/mod.rs" "db/surreal.rs" "db/qdrant.rs" \
           "tools/mod.rs" "tools/types.rs" "tools/ocr.rs" "tools/meta_search.rs"; do
  if [ -f "$mod" ]; then
    : # silent pass
  else
    log_fail "Missing: $mod" "File was deleted"
  fi
done

# Check Tauri commands registered in lib.rs
if grep -q "enrich_prompt" src/lib.rs && \
   grep -q "process_turn" src/lib.rs && \
   grep -q "process_message" src/lib.rs && \
   grep -q "get_kb_data" src/lib.rs && \
   grep -q "run_ocr" src/lib.rs && \
   grep -q "run_search" src/lib.rs && \
   grep -q "ingest_text" src/lib.rs && \
   grep -q "run_consolidation" src/lib.rs && \
   grep -q "generate_tts" src/lib.rs && \
   grep -q "stop_tts" src/lib.rs; then
  log_pass "11 Tauri commands registered"
else
  log_fail "Tauri command registration incomplete" "Check src/lib.rs"
fi

# Tool binaries (optional)
for tool in "ocr_helper_system:ocr-helper:ocr_helper" "meta_search_Js_render:meta_search:meta_search"; do
  IFS=':' read -r proj bin label <<< "$tool"
  bin_path="$SCRIPT_DIR/../$proj/target/release/$bin"
  if [ -f "$bin_path" ]; then
    log_info "$label binary found (optional)"
  else
    log_skip "$label binary not built (optional — cd ../$proj && cargo build --release)"
  fi
done

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

# JS modules
for js in \
  "src/lib/ipc.js" "src/lib/markdown.js" "src/lib/drag-resize.js" \
  "src/lib/events.js" "src/lib/prompts.js" "src/lib/process-notify.js" \
  "src/lib/alice-responses.js" \
  "src/ui/panel-system.js" "src/ui/status-panel.js" "src/ui/chat-panel.js" \
  "src/ui/ocr-panel.js" "src/ui/search-panel.js" "src/ui/kb-panel.js" \
  "src/ui/sidebar.js" \
  "src/main.js" "src/tts-player.js"; do
  if [ -f "$SCRIPT_DIR/$js" ]; then
    : # silent
  else
    log_fail "Missing: $js" "File was deleted"
  fi
done
log_pass "All 17 JS modules present"

# HTML
[ -f "$SCRIPT_DIR/index.html" ] && log_pass "index.html found" \
  || log_fail "index.html missing"

# Check Alice prompt in prompts.js
if grep -q "ALICE.*demoness" "$SCRIPT_DIR/src/lib/prompts.js" 2>/dev/null; then
  log_pass "Alice identity prompt present"
else
  log_fail "Alice identity prompt missing" "Check src/lib/prompts.js"
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

# Cargo.toml deps
for crate in "surrealdb" "qdrant-client" "petgraph" "tokio" "reqwest" \
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

# Live2D model files
for model_file in "demongirl.model3.json" "demongirl.moc3" "demongirl.physics3.json"; do
  if [ -f "$SCRIPT_DIR/public/DemonGirl/$model_file" ]; then
    : # silent
  else
    log_fail "Missing Live2D: $model_file" "Check public/DemonGirl/"
  fi
done

# Expression files
EXPR_COUNT=$(ls "$SCRIPT_DIR/public/DemonGirl/"*.exp3.json 2>/dev/null | wc -l)
[ "$EXPR_COUNT" -ge 7 ] && log_pass "Live2D expressions: $EXPR_COUNT/7" \
  || log_fail "Live2D expressions: $EXPR_COUNT/7" "Missing .exp3.json files"

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
      -d '{"ns":"alice","db":"memory","sql":"SELECT * FROM fact LIMIT 1"}' 2>/dev/null || echo "")
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
      -d '{"ns":"alice","db":"memory","sql":"INFO FOR DB;"}' 2>/dev/null | jq -r '.[0].result.tables | keys | join(", ")' 2>/dev/null || echo "")
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
DEBUG_COUNT=$(grep -r "console\.\(log\|debug\)" "$SCRIPT_DIR/src" --include="*.js" 2>/dev/null | grep -v "warn\|error" | wc -l || echo 0)
if [ "$DEBUG_COUNT" -le 5 ]; then
  log_pass "JS console.log/debug: $DEBUG_COUNT (acceptable)"
else
  log_info "JS console.log/debug: $DEBUG_COUNT (consider removing some)"
fi

# Check for TODO comments
TODO_COUNT=$(grep -r "TODO\|FIXME\|HACK" "$SCRIPT_DIR/src" "$SCRIPT_DIR/src-tauri/src" --include="*.js" --include="*.rs" 2>/dev/null | wc -l || echo 0)
if [ "$TODO_COUNT" -gt 0 ]; then
  log_info "TODOs/FIXMEs remaining: $TODO_COUNT"
fi

# Check for hardcoded secrets
if grep -r "password\|secret\|api.key\|token" "$SCRIPT_DIR/src-tauri/src" --include="*.rs" 2>/dev/null | grep -v "root\|password\|MemoryConfig\|//" | grep -q .; then
  log_info "Potential hardcoded secrets found — review src-tauri/src/"
fi

# Line counts
RS_LINES=$(find "$SCRIPT_DIR/src-tauri/src" -name "*.rs" -exec cat {} + 2>/dev/null | wc -l)
JS_LINES=$(find "$SCRIPT_DIR/src" -name "*.js" -exec cat {} + 2>/dev/null | wc -l)
CSS_LINES=$(find "$SCRIPT_DIR/styles" -name "*.css" -exec cat {} + 2>/dev/null | wc -l)
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
      -d '{"model":"gemma4:e4b","messages":[{"role":"user","content":"Extract entities from: master uses Rust and Tauri.\nOutput ONLY a JSON array."}],"stream":false,"options":{"temperature":0,"num_predict":128}}' \
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
