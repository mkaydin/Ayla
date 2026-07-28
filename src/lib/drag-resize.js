// ── Drag and resize primitives ─────────────────────────────

export function makeDraggable(panel, headerEl) {
  let dragging = false, offX = 0, offY = 0;

  const onDown = (e) => {
    if (e.target.closest('.panel-btn')) return;
    e.preventDefault();
    dragging = true;
    offX = e.clientX - panel.x;
    offY = e.clientY - panel.y;
    panel.el.style.transition = 'none';
    panel.focus();
    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onUp);
  };

  const onMove = (e) => {
    if (!dragging) return;
    panel.moveTo(e.clientX - offX, e.clientY - offY);
  };

  const onUp = () => {
    dragging = false;
    panel.el.style.transition = '';
    document.removeEventListener('pointermove', onMove);
    document.removeEventListener('pointerup', onUp);
  };

  headerEl.addEventListener('pointerdown', onDown);
}

export function makeResizable(panel, resizeEl) {
  let resizing = false;

  const onDown = (e) => {
    e.preventDefault();
    e.stopPropagation();
    resizing = true;
    panel.el.style.transition = 'none';
    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onUp);
  };

  const onMove = (e) => {
    if (!resizing) return;
    const w = e.clientX - panel.x;
    const h = e.clientY - panel.y;
    panel.resizeTo(Math.max(panel.minWidth, w), Math.max(panel.minHeight, h));
  };

  const onUp = () => {
    resizing = false;
    panel.el.style.transition = '';
    document.removeEventListener('pointermove', onMove);
    document.removeEventListener('pointerup', onUp);
  };

  resizeEl.addEventListener('pointerdown', onDown);
}
