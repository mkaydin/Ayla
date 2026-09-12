> Historical design record. Runtime and setup instructions are superseded by [README](../README.md) and the [production readiness plan](PRODUCTION_READINESS_PLAN.md).

# UI Optimization & Polish Plan

## 1. Fix White/Black Form Elements

**Root cause:** WebKitGTK (Tauri's webview on Linux) uses native GTK form controls for `<select>` elements. CSS `option` styling is ignored — dropdown menus use system GTK theme.

**Fixes applied:**
- Created `styles/forms.css` — global dark theme reset for all `input`, `select`, `textarea`
- Custom dropdown arrow via SVG background-image
- `select option { background/color }` (works on most platforms, may fail on GTK)
- Removed browser default search input decorations

**If dropdowns still show white:** This is unfixable with CSS alone on WebKitGTK. Workaround: replace `<select>` with custom JS dropdown component.

---

## 2. Render Speed Optimization

### 2.1 — Chat Streaming (biggest lag source)

**Problem:** On every streaming token, `innerHTML = renderMarkdown(raw)` re-parses and re-renders the ENTIRE response. For a 2000-token response, this runs 2000 times — each time parsing increasingly large markdown.

**Fix:** Debounce and use `requestAnimationFrame`:
```javascript
let renderScheduled = false;
let pendingRaw = '';

function scheduleRender() {
  if (renderScheduled) return;
  renderScheduled = true;
  requestAnimationFrame(() => {
    thinkDiv.innerHTML = renderMarkdown(pendingRaw);
    renderScheduled = false;
  });
}

// In streaming loop, replace:
// thinkDiv.innerHTML = renderMarkdown(displayText);
// with:
pendingRaw = displayText;
scheduleRender();
```

**Expected:** Reduces re-renders from ~2000 to ~60 (once per frame at 60fps). 97% reduction in DOM operations.

### 2.2 — Panel Drag

**Problem:** `moveTo()` sets `el.style.left` and `el.style.top` on every pointermove, triggering layout recalculations.

**Fix:** Use CSS `transform` instead:
```javascript
moveTo(x, y) {
  this.x = Math.max(-this.width + 40, Math.min(x, window.innerWidth - 40));
  this.y = Math.max(0, Math.min(y, window.innerHeight - 32));
  this.el.style.transform = `translate(${this.x}px, ${this.y}px)`;
}
```

Also add `will-change: transform` to `.panel` CSS.

**Expected:** GPU-composited movement, no layout thrashing.

### 2.3 — Status Panel Health Checks

**Problem:** `checkAllServices()` fetches 7 URLs every 30s, each with 5s timeout. This can queue up to 35s of network requests.

**Fix:** Stagger health checks — check one service every 5 seconds instead of all at once:
```javascript
let checkIndex = 0;
setInterval(() => {
  checkService(SERVICES[checkIndex % SERVICES.length]);
  checkIndex++;
}, 5000);
```

**Expected:** Constant light network load instead of periodic spikes.

### 2.4 — KB Graph Rendering

**Problem:** Canvas redraws entire graph on every interaction (drag, zoom, filter change).

**Fix:**
- Skip redraw if panel is minimized or hidden
- Only render visible nodes (viewport culling)
- Use `requestAnimationFrame` for graph updates

### 2.5 — Panel System

**Problem:** `saveAllPanelStates()` writes to `localStorage` on every drag/resize via `beforeunload`. This is actually fine (only on page close).

**Fix:** Throttle position saves to localStorage to once per second during drag/resize.

---

## 3. Logo Design

### Window Icon

Create `public/icon.svg` — a demoness sigil:

```svg
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#1a1025"/>
      <stop offset="100%" stop-color="#0d0612"/>
    </linearGradient>
    <linearGradient id="sigil" x1="0" y1="1" x2="1" y2="0">
      <stop offset="0%" stop-color="#7c3aed"/>
      <stop offset="100%" stop-color="#3b82f6"/>
    </linearGradient>
  </defs>
  <!-- Background circle -->
  <circle cx="256" cy="256" r="248" fill="url(#bg)" stroke="url(#sigil)" stroke-width="8"/>
  <!-- Stylized "A" sigil -->
  <path d="M256 80 L380 400 L320 400 L290 310 L222 310 L192 400 L132 400 Z
           M256 160 L236 230 L276 230 Z"
        fill="url(#sigil)" opacity="0.9"/>
  <!-- Demon horns -->
  <path d="M170 150 Q130 60 80 80 Q110 100 140 170" fill="none" stroke="url(#sigil)" stroke-width="6" stroke-linecap="round"/>
  <path d="M342 150 Q382 60 432 80 Q402 100 372 170" fill="none" stroke="url(#sigil)" stroke-width="6" stroke-linecap="round"/>
</svg>
```

Generate PNG icons at required sizes using `convert` (ImageMagick) or `rsvg-convert`:
```bash
# From SVG to PNG icons
for size in 32 128 256 512; do
  rsvg-convert -w $size -h $size public/icon.svg > src-tauri/icons/${size}x${size}.png
done
# Also create @2x versions
cp src-tauri/icons/128x128.png src-tauri/icons/128x128@2x.png
```

### App Title Bar

Change `tauri.conf.json` title from `"DemonGirl — Live2D"` to `"ALICE"`.

---

## 4. UI Polish

### 4.1 — Chat Panel
- Add smooth scroll animation to message container
- Show typing indicator (animated dots) while waiting for first token
- Add message timestamps on hover (existing plan, not implemented)
- Reduce chat sidebar width to 180px (was 200px)

### 4.2 — Status Panel
- Add collapse animation (height transition)
- Show memory stats as compact badges instead of text row
- Add "Copy log" button to copy status log to clipboard

### 4.3 — Sidebar
- Add tooltip labels on hover (already in CSS, verify it works)
- Add "Dock sidebar" toggle to lock it visible

### 4.4 — General
- Add panel open/close animations (fade + scale)
- Add loading spinner for initial app boot
- Add notification toast for completed background tasks
- Add keyboard shortcut cheatsheet (Ctrl+? to toggle)

---

## 5. Implementation Order

### Phase A — Dropdown Fix (15 min)
```
1. Create styles/forms.css ✓ (done)
2. Add to index.html ✓ (done)
3. If native selects still white on GTK → replace with JS custom select
```

### Phase B — Chat Optimization (30 min)
```
4. Debounce markdown rendering with rAF
5. Smooth scroll on new messages
```

### Phase C — Panel Optimization (30 min)
```
6. Transform-based panel positioning
7. Stagger health checks
8. Throttle position saves
9. KB graph skip-when-hidden
```

### Phase D — Logo & Branding (20 min)
```
10. Create icon.svg
11. Generate PNG icons
12. Update tauri.conf.json title
```

### Phase E — Polish (30 min)
```
13. Animations (panel open/close fade)
14. Tooltips verification
15. Notification toast component
```

**Total: ~2 hours**
