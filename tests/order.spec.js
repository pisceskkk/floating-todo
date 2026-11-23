import { test, expect } from '@playwright/test';
import { newDocument, commitView, projectView, dateInZone } from '../src/sync-model.js';

const CACHE_KEY = 'floating-todo/v3-preview';
const item = (id, extra = {}) => ({ id, title: '任务 ' + id, detail: '', time: null, completed: false, createdAt: 1, projectId: 'p', tagIds: ['t'], ...extra });
const baseView = () => ({
  schemaVersion: 3, selectedDay: 'today', projects: [], tags: [], colorCounters: { projects: 0, tags: 0 },
  itemsByDay: { today: [], tomorrow: [], dayAfterTomorrow: [] }, recurringItems: [],
  settings: { memoEnabled: false }, memo: { text: '', expanded: false }, lastActiveDate: dateInZone(),
});

function buildDoc(lists = {}) {
  const doc = newDocument();
  const base = baseView();
  const view = {
    ...base,
    itemsByDay: { today: lists.today || [], tomorrow: lists.tomorrow || [], dayAfterTomorrow: lists.dayAfterTomorrow || [] },
    recurringItems: lists.recurring || [],
    projects: [{ id: 'p', name: '工作', color: '#397fe5' }, { id: 'hidden', name: '生活', color: '#4a9b74' }],
    tags: [{ id: 't', name: '重要', color: '#ae65d6' }],
  };
  commitView(doc, base, view);
  for (const [groupIndex, list] of [view.itemsByDay.today, view.itemsByDay.tomorrow, view.itemsByDay.dayAfterTomorrow, view.recurringItems].entries()) {
    list.forEach((t, i) => {
      const r = doc.revisions.find(rev => rev.entityType === 'order' && rev.entityId === t.id);
      if (r) r.payload.position = i + (2 - groupIndex) * 1000000;
    });
  }
  return doc;
}

async function boot(page, lists = {}) {
  const rawDoc = JSON.stringify(buildDoc(lists));
  await page.addInitScript(({ key, rawDoc }) => {
    if (!localStorage.getItem(key)) localStorage.setItem(key, rawDoc);
  }, { key: CACHE_KEY, rawDoc });
  await page.goto('/'); await expect(page.locator('#main-input')).toBeVisible();
}
async function read(page) {
  const raw = await page.evaluate(key => localStorage.getItem(key), CACHE_KEY);
  return projectView(JSON.parse(raw), baseView());
}
const order = (page, list = '.list') => page.locator(`${list} > .row`).evaluateAll(rows => rows.map(row => row.dataset.row));
async function drag(page, id, targetId, { list = '.list', title = false, drop = true } = {}) {
  const source = await page.locator(`${list} > [data-row="${id}"] ${title ? '.title' : '.drag-handle'}`).boundingBox();
  const target = await page.locator(`${list} > [data-row="${targetId}"]`).boundingBox();
  await page.mouse.move(source.x + source.width / 2, source.y + source.height / 2);
  await page.mouse.down();
  await page.mouse.move(source.x + source.width / 2, target.y + target.height - 3, { steps: 8 });
  await expect(page.locator('.drag-ghost')).toHaveCount(1);
  if (drop) await page.mouse.up();
}

test('ordinary title dragging persists order, metadata, completed grouping and editing', async ({ page }) => {
  await boot(page, { today: ['a','b','c'].map(id => item(id)).concat(item('done', { completed: true })) });
  await drag(page, 'a', 'c', { title: true });
  expect(await order(page)).toEqual(['b','c','a','done']);
  expect((await read(page)).itemsByDay.today.find(t => t.id === 'a')).toMatchObject({ projectId: 'p', tagIds: ['t'] });
  await expect(page.locator('#main-input')).toHaveValue('');
  await page.reload(); expect(await order(page)).toEqual(['b','c','a','done']);
  await page.locator('[data-row="b"] .body').click(); await expect(page.locator('#main-input')).toHaveValue('任务 b +工作 #重要');
});

test('filtered pointer and keyboard sorting preserves hidden slots and works across dates', async ({ page }) => {
  const tasks = [item('a'), item('hidden', { projectId: 'hidden', tagIds: [] }), item('b'), item('c')];
  await boot(page, { today: tasks, tomorrow: tasks.map(t => ({ ...t, id: t.id + '2' })) });
  await page.locator('[data-class-ui="filter-projects"]').click(); await page.locator('[data-filter-project="p"]').click();
  await drag(page, 'a', 'c');
  expect((await read(page)).itemsByDay.today.map(t => t.id)).toEqual(['b','hidden','c','a']);
  await page.locator('[data-row="a"] .drag-handle').focus(); await page.keyboard.press('Alt+ArrowUp');
  expect((await read(page)).itemsByDay.today.map(t => t.id)).toEqual(['b','hidden','a','c']);
  await page.locator('[data-act="day"][data-day="tomorrow"]').click();
  await page.locator('[data-row="a2"] .drag-handle').focus(); await page.keyboard.press('Alt+ArrowDown');
  expect((await read(page)).itemsByDay.tomorrow.map(t => t.id)).toEqual(['b2','hidden2','a2','c2']);
  await page.locator('[data-class-ui="clear"]').first().click();
  expect(await order(page)).toEqual(['b2','hidden2','a2','c2']);
});

test('recurring sorting stays within its own list and completion group', async ({ page }) => {
  await boot(page, { today: [item('normal')], recurring: [item('a'),item('b'),item('c'),item('done', { completed: true })] });
  await drag(page, 'a', 'c', { list: '.recurring' });
  expect(await order(page, '.recurring')).toEqual(['b','c','a','done']);
  await page.locator('[data-row="done"] .drag-handle').focus(); await page.keyboard.press('Alt+ArrowUp');
  expect((await read(page)).recurringItems.map(t => t.id)).toEqual(['b','c','a','done']);
  expect((await read(page)).itemsByDay.today.map(t => t.id)).toEqual(['normal']);
});

test('Escape and resize cancel in-progress ordering without saving it', async ({ page }) => {
  await boot(page, { today: ['a','b','c'].map(id => item(id)) });
  await drag(page, 'a', 'c', { drop: false });
  await page.keyboard.press('Escape'); await page.mouse.up();
  expect(await order(page)).toEqual(['a','b','c']); await expect(page.locator('.drag-ghost')).toHaveCount(0);
  await drag(page, 'a', 'c', { drop: false });
  await page.setViewportSize({ width: 280, height: 560 }); await page.mouse.up();
  expect((await read(page)).itemsByDay.today.map(t => t.id)).toEqual(['a','b','c']);
  await expect(page.locator('.drag-ghost')).toHaveCount(0);
});

test('long lists scroll at the drag edge and compact mode supports sorting', async ({ page }) => {
  await boot(page, { today: Array.from({ length: 30 }, (_, i) => item(String(i))) });
  const source = await page.locator('[data-row="0"] .drag-handle').boundingBox();
  const list = await page.locator('.list').boundingBox();
  await page.mouse.move(source.x + 5, source.y + 5); await page.mouse.down();
  await page.mouse.move(source.x + 5, list.y + list.height - 3, { steps: 8 });
  await expect.poll(() => page.locator('.list').evaluate(el => el.scrollTop)).toBeGreaterThan(20);
  await page.keyboard.press('Escape'); await page.mouse.up();
  await page.setViewportSize({ width: 250, height: 560 });
  await expect(page.locator('html')).toHaveClass(/compact/);
  await page.locator('.list').evaluate(el => { el.scrollTop = 0; });
  await drag(page, '0', '1');
  expect((await read(page)).itemsByDay.today.slice(0, 2).map(t => t.id)).toEqual(['1','0']);
});
