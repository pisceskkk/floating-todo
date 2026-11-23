import { PANEL_MIN, PANEL_DEFAULT, BAR_SIZE, CAPTURE_SIZE, STACK_SIZE, TASKS_SIZE, floatingBounds, isCompact, anchoredBounds } from './window-layout.js';

export function createWindowController({ api, moveToScreen, onChange, onError, mobile = false }) {
  const native = mobile ? null : api?.getCurrentWindow?.();
  let mode = 'panel';
  let transitioning = false;
  let lastFullSize = { ...PANEL_DEFAULT };
  let task = Promise.resolve();
  let direction = { horizontal: 'left', vertical: 'down' };

  function observeSize() {
    if (mobile || mode !== 'panel' || transitioning) return;
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

  function setMode(next) {
    if (mobile) return Promise.resolve(next === 'panel');
    // Serialize requests: native resize events can arrive between size and
    // position updates, and must never switch a mini window to compact mode.
    task = task.then(async () => {
      if (next === mode && (next !== 'panel' || !isCompact({ width: innerWidth, height: innerHeight }))) return true;
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

  return {
    get mode() { return mode; },
    get direction() { return direction; },
    get floating() { return mode !== 'panel'; },
    get transitioning() { return transitioning; },
    observeSize, setMode,
  };
}
