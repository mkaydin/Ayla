// ── Drag and resize primitives ─────────────────────────────

export function makeDraggable(panel, headerEl) {
  let dragging = false, offX = 0, offY = 0;
  let frame = 0, nextX = 0, nextY = 0;

  const onDown = (e) => {
    if (e.target.closest('.panel-btn')) return;
    e.preventDefault();
    dragging = true;
    nextX = panel.x;
    nextY = panel.y;
    offX = e.clientX - panel.x;
    offY = e.clientY - panel.y;
    panel.el.style.transition = 'none';
    panel.el.classList.add('is-dragging');
    panel.focus();
    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onUp);
    document.addEventListener('pointercancel', onUp);
  };

  const onMove = (e) => {
    if (!dragging) return;
    nextX = e.clientX - offX;
    nextY = e.clientY - offY;
    if (!frame) {
      frame = requestAnimationFrame(() => {
        frame = 0;
        const x = Math.max(-panel.width + 40, Math.min(nextX, window.innerWidth - 40));
        const y = Math.max(0, Math.min(nextY, window.innerHeight - 32));
        panel.el.style.transform = `translate3d(${x - panel.x}px, ${y - panel.y}px, 0)`;
      });
    }
  };

  const onUp = () => {
    if (frame) {
      cancelAnimationFrame(frame);
      frame = 0;
    }
    panel.el.style.transform = '';
    panel.moveTo(nextX, nextY);
    dragging = false;
    panel.el.style.transition = '';
    panel.el.classList.remove('is-dragging');
    document.removeEventListener('pointermove', onMove);
    document.removeEventListener('pointerup', onUp);
    document.removeEventListener('pointercancel', onUp);
  };

  headerEl.addEventListener('pointerdown', onDown);
}

export function makeResizable(panel, resizeEl) {
  let resizing = false;
  let frame = 0, nextW = 0, nextH = 0;

  const onDown = (e) => {
    e.preventDefault();
    e.stopPropagation();
    resizing = true;
    nextW = panel.width;
    nextH = panel.height;
    panel.el.style.transition = 'none';
    panel.el.classList.add('is-resizing');
    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onUp);
    document.addEventListener('pointercancel', onUp);
  };

  const onMove = (e) => {
    if (!resizing) return;
    nextW = Math.max(panel.minWidth, e.clientX - panel.x);
    nextH = Math.max(panel.minHeight, e.clientY - panel.y);
    if (!frame) {
      frame = requestAnimationFrame(() => {
        frame = 0;
        panel.resizeTo(nextW, nextH);
      });
    }
  };

  const onUp = () => {
    if (frame) {
      cancelAnimationFrame(frame);
      frame = 0;
      panel.resizeTo(nextW, nextH);
    }
    resizing = false;
    panel.el.style.transition = '';
    panel.el.classList.remove('is-resizing');
    document.removeEventListener('pointermove', onMove);
    document.removeEventListener('pointerup', onUp);
    document.removeEventListener('pointercancel', onUp);
  };

  resizeEl.addEventListener('pointerdown', onDown);
}
