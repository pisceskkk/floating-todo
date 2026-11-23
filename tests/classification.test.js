import test from 'node:test';
import assert from 'node:assert/strict';
import { ensureCategory, updateCategory, categoryCount, removeCategory, parseClassification, resolveClassification, editText, activeToken, suggestions, matchesFilters, PALETTE } from '../src/classification.js';

const state = () => ({ selectedDay: 'today', projects: [], tags: [], colorCounters: { projects: 0, tags: 0 }, itemsByDay: { today: [], tomorrow: [], dayAfterTomorrow: [] }, recurringItems: [], settings: { opacity: .5 }, memo: { text: '' }, lastActiveDate: '2026-10-19' });
const task = (id, extra = {}) => ({ id, title: '旧任务', completed: false, time: '15:00', streak: 5, ...extra });

test('quick syntax strips metadata, last project wins and no provisional categories are created', () => {
  const s = state();
  const parsed = parseClassification('明天 15:00 提交设计稿 +旧项目 +官网改版 #设计 #重要 #设计');
  assert.equal(parsed.title, '明天 15:00 提交设计稿'); assert.equal(s.tags.length, 0);
  const links = resolveClassification(s, parsed.tokens);
  assert.equal(s.projects.length, 1); assert.equal(s.projects[0].name, '官网改版'); assert.equal(links.tagIds.length, 2);
  assert.deepEqual(resolveClassification(s, parsed.tokens), links);
});

test('quoted category names roundtrip through editing, including spaces, quotes and numeric names', () => {
  const s = state();
  const p = ensureCategory(s, 'projects', 'Website "Redesign"');
  const tags = ['重要 事项', '123', 'C#', '研发/设计'].map(n => ensureCategory(s, 'tags', n));
  const t = task('a', { projectId: p.id, tagIds: tags.map(t => t.id) });
  const parsed = parseClassification(editText(s, t));
  assert.equal(parsed.title, t.title); assert.deepEqual(resolveClassification(s, parsed.tokens), { projectId: p.id, tagIds: t.tagIds });
});

test('URLs, paths, C#, arithmetic and incomplete tokens remain title text', () => {
  for (const text of ['查看 https://example.com/a#b + https://a.com', 'C# C++ a+b /tmp/a C:\\work\\a', '计算 1 +2 + 3', '待办 # +', '待办 +"尚未完成']) {
    assert.equal(parseClassification(text).title, text);
  }
  assert.equal(parseClassification('任务 #重要 +项目').tokens.length, 2);
  assert.equal(parseClassification('任务 #123').tokens[0].name, '123');
  assert.equal(activeToken('任务 #123', 7).query, '123');
});

test('autocomplete tracks cursor and quoted names, never changes persisted state', () => {
  const s = state(); ensureCategory(s, 'tags', '工作'); ensureCategory(s, 'tags', '日常工作');
  const before = JSON.stringify(s);
  const active = activeToken('写报告 #工 后续', 6);
  assert.equal(active.query, '工'); assert.equal(suggestions(s, active)[0].name, '工作');
  assert.equal(suggestions(s, active).at(-1).create, true);
  assert.equal(activeToken('+"Website Re', 12).query, 'Website Re');
  assert.equal(activeToken('https://a#b', 11), null);
  assert.equal(JSON.stringify(s), before);
});

test('colors cycle separately and deletion does not recolor existing entries', () => {
  const s = state();
  const tags = Array.from({ length: PALETTE.length + 1 }, (_, i) => ensureCategory(s, 'tags', '标签' + i));
  assert.equal(tags[0].color, tags.at(-1).color);
  assert.equal(ensureCategory(s, 'projects', '工作').color, PALETTE[0]);
  const color = tags[1].color; removeCategory(s, 'tags', tags[0].id); assert.equal(tags[1].color, color);
  assert.equal(ensureCategory(s, 'tags', '新标签').color, PALETTE[1]);
});

test('rename and recolor preserve relationships; duplicate and empty names are rejected', () => {
  const s = state(), p = ensureCategory(s, 'projects', 'Work'); ensureCategory(s, 'projects', 'Other');
  s.itemsByDay.today.push(task('a', { projectId: p.id, tagIds: [] }));
  updateCategory(s, 'projects', p.id, { name: '工作', color: '#aabbcc' });
  assert.equal(s.itemsByDay.today[0].projectId, p.id); assert.equal(p.color, '#aabbcc');
  assert.throws(() => updateCategory(s, 'projects', p.id, { name: ' other ' }));
  assert.throws(() => updateCategory(s, 'projects', p.id, { name: ' ' }));
});

test('delete label retains tasks across every date and habits; project supports both choices', () => {
  for (const deleteTasks of [false, true]) {
    const s = state(), p = ensureCategory(s, 'projects', '工作'), tag = ensureCategory(s, 'tags', '重要');
    for (const day of Object.keys(s.itemsByDay)) s.itemsByDay[day].push(task(day, { projectId: p.id, tagIds: [tag.id] }));
    s.recurringItems.push(task('daily', { projectId: p.id, tagIds: [tag.id] }));
    assert.equal(categoryCount(s, 'projects', p.id), 4);
    removeCategory(s, 'tags', tag.id); assert.equal(s.recurringItems.length, 1); assert.deepEqual(s.recurringItems[0].tagIds, []);
    removeCategory(s, 'projects', p.id, deleteTasks);
    assert.equal(s.recurringItems.length, deleteTasks ? 0 : 1);
    for (const list of Object.values(s.itemsByDay)) { assert.equal(list.length, deleteTasks ? 0 : 1); if (!deleteTasks) assert.equal(list[0].projectId, null); }
  }
});

test('filters intersect project with any selected tag and support no-project', () => {
  const t = task('a', { projectId: 'p', tagIds: ['a', 'b'] });
  assert.equal(matchesFilters(t, { project: 'p', tags: ['b', 'c'] }), true);
  assert.equal(matchesFilters(t, { project: 'x', tags: ['b'] }), false);
  assert.equal(matchesFilters(t, { project: 'p', tags: ['c'] }), false);
  assert.equal(matchesFilters({ ...t, projectId: null }, { project: 'none', tags: [] }), true);
});
