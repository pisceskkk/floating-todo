import { PANEL_MIN, PANEL_DEFAULT, BAR_SIZE, CAPTURE_SIZE, STACK_SIZE, TASKS_SIZE, floatingBounds, isCompact, anchoredBounds, southwestResize } from './window-layout.js';

export function createWindowController({ api, moveToScreen, onChange, onError }) {
  const native = api?.getCurrentWindow?.();
  let mode = 'panel';
  let transitioning = false;
  let lastFullSize = { ...PANEL_DEFAULT };
  let resize = null;
  let resizeTask = Promise.resolve();
  let task = Promise.resolve();
  let direction = { horizontal: 'left', vertical: 'down' };

  function observeSize() {
    if (mode !== 'panel' || transitioning || resize) return;
    const size = { width: innerWidth, height: innerHeight };
    if (!isCompact(size)) lastFullSize = size;
  }

  async function frame() {
    if (!native) return { x: 0, y: 0, width: innerWidth, height: innerHeight, scale: 1 };
    const [position, size, scale] = await Promise.all([native.outerPosition(), native.innerSize(), native.scaleFactor()]);
    return { x: position.x, y: position.y, width: size.width, height: size.height, scale };
  }

  async function setBounds(bounds) {
    if (!native) return;
    await native.setSize(new api.PhysicalSize(bounds.width, bounds.height));
    await moveToScreen(bounds.x, bounds.y);
  }

  function releaseResize() {
    const current = resize;
    resize = null;
    if (current?.handle.hasPointerCapture(current.pointerId)) current.handle.releasePointerCapture(current.pointerId);
    document.documentElement.classList.remove('window-resizing');
  }

  function setMode(next) {
    // Serialize requests: native resize events can arrive between size and
    // position updates, and must never switch a mini window to compact mode.
    task = task.then(async () => {
      if (next === mode && (next !== 'panel' || !isCompact({ width: innerWidth, height: innerHeight }))) return true;
      releaseResize();
      await resizeTask;
      observeSize();
      const previous = mode;
      const previousDirection = direction;
      let before;
      transitioning = true;
      try {
        before = await frame();
        const monitor = native ? await api.currentMonitor() : null;
        const area = monitor?.workArea || (monitor && { position: monitor.position, size: monitor.size });
        const size = next === 'panel' ? lastFullSize : next === 'mini' ? BAR_SIZE : next === 'stack' ? STACK_SIZE : next === 'tasks' ? TASKS_SIZE : CAPTURE_SIZE;
        let anchor = before;
        if (previous !== 'panel' && previous !== 'mini') {
          const width = Math.round(BAR_SIZE.width * before.scale);
          const height = Math.round(BAR_SIZE.height * before.scale);
          anchor = { ...before, width, height,
            x: direction.horizontal === 'left' ? before.x + before.width - width : before.x,
            y: direction.vertical === 'up' ? before.y + before.height - height : before.y };
        }
        let bounds;
        if (next === 'panel' || previous === 'panel' && next === 'mini') {
          bounds = anchoredBounds(anchor, size, area);
        } else {
          const result = floatingBounds(anchor, size, area, previous === 'stack' || previous === 'tasks' || previous === 'quick' ? direction : null);
          bounds = result.bounds;
          direction = result.direction;
        }
        mode = next;
        onChange();
        if (native) {
          const min = next === 'panel' ? PANEL_MIN : BAR_SIZE;
          await native.setMinSize(new api.LogicalSize(min.width, min.height));
          await native.setResizable(next === 'panel');
          await setBounds(bounds);
        }
        return true;
      } catch (error) {
        mode = previous;
        direction = previousDirection;
        if (native && before) {
          try {
            const min = previous === 'panel' ? PANEL_MIN : BAR_SIZE;
            await native.setMinSize(new api.LogicalSize(min.width, min.height));
            await native.setResizable(previous === 'panel');
            await setBounds(before);
          } catch (restoreError) { console.error('Failed to restore window bounds', restoreError); }
        }
        onError(error);
        return false;
      } finally {
        transitioning = false;
        onChange();
      }
    });
    return task;
  }

  function attachResize(handle) {
    handle.addEventListener('pointerdown', async (event) => {
      if (event.button !== 0 || mode !== 'panel' || transitioning || resize) return;
      if (!native) return;
      event.preventDefault();
      observeSize();
      const current = { handle, pointerId: event.pointerId, ready: false, pending: false, running: false };
      resize = current;
      handle.setPointerCapture(event.pointerId);
      document.documentElement.classList.add('window-resizing');
      try {
        current.frame = await frame();
        current.bounds = current.frame;
        current.start = { x: current.frame.x + event.clientX * current.frame.scale, y: current.frame.y + event.clientY * current.frame.scale };
        current.ready = true;
        if (resize === current && current.pending) sampleResize(current);
      } catch (error) { releaseResize(); onError(error); }
    });
    handle.addEventListener('pointermove', (event) => {
      if (resize?.pointerId !== event.pointerId || resize.finishing) return;
      if (!(event.buttons & 1)) { finishResize(event); return; }
      resize.pending = true;
      sampleResize(resize);
    });
    handle.addEventListener('pointerup', finishResize);
    handle.addEventListener('pointercancel', releaseResize);
    handle.addEventListener('lostpointercapture', () => { if (!resize?.finishing) releaseResize(); });
    window.addEventListener('blur', releaseResize);
    document.addEventListener('keydown', (event) => {
      if (event.key === 'Escape' && resize) { event.preventDefault(); releaseResize(); }
    });
  }

  function finishResize(event) {
    if (resize?.pointerId !== event.pointerId) return;
    resize.finishing = true;
    resize.pending = true;
    sampleResize(resize);
  }

  function sampleResize(current) {
    if (!current.ready || current.running || resize !== current) return;
    current.running = true;
    resizeTask = (async () => {
      try {
        while (resize === current && current.pending) {
          current.pending = false;
          const pointer = await api.cursorPosition();
          if (resize !== current) break;
          const result = southwestResize(current.frame, current.start, pointer);
          if (['x', 'y', 'width', 'height'].some(key => result.bounds[key] !== current.bounds[key])) {
            await setBounds(result.bounds);
            current.bounds = result.bounds;
          }
          if (resize !== current) break;
          if (result.collapse) {
            releaseResize();
            // Queue after this sample completes to avoid racing native geometry.
            setMode('mini');
            break;
          }
        }
      } catch (error) { releaseResize(); onError(error); }
      finally {
        current.running = false;
        if (resize === current && current.finishing) releaseResize();
      }
    })();
  }

  return {
    get mode() { return mode; },
    get direction() { return direction; },
    get floating() { return mode !== 'panel'; },
    get transitioning() { return transitioning; },
    observeSize, setMode, attachResize,
  };
}
