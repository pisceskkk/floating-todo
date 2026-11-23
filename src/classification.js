// Shared by v3 view projection, quick capture and classification settings.
export const SCHEMA_VERSION = 3;
export const PALETTE = ["#397fe5", "#ae65d6", "#d7628e", "#da8740", "#4a9b74", "#399cac", "#8675d4", "#b29a38"];
export const DAY_KEYS = ["today", "tomorrow", "dayAfterTomorrow"];
export const nameKey = (name) => String(name ?? "").trim().normalize("NFKC").toLowerCase();
export const validColor = (color) => /^#[0-9a-f]{6}$/i.test(color);
const newId = () => crypto.randomUUID();
const cleanName = (value) => String(value ?? "").trim().replace(/\s+/g, " ");

export function allTasks(state) {
  return [...DAY_KEYS.flatMap((day) => state.itemsByDay[day]), ...state.recurringItems];
}

export function ensureCategory(state, kind, value) {
  const name = cleanName(value);
  if (!name) throw new Error("名称不能为空");
  const found = state[kind].find((entry) => nameKey(entry.name) === nameKey(name));
  if (found) return found;
  const entry = { id: newId(), name, color: PALETTE[state.colorCounters[kind] % PALETTE.length] };
  state.colorCounters[kind]++;
  state[kind].push(entry);
  return entry;
}

export function updateCategory(state, kind, id, changes) {
  const entry = state[kind].find((value) => value.id === id);
  if (!entry) throw new Error("分类已不存在");
  if (changes.name !== undefined) {
    const name = cleanName(changes.name);
    if (!name) throw new Error("名称不能为空");
    if (state[kind].some((value) => value.id !== id && nameKey(value.name) === nameKey(name))) throw new Error("已有同名分类");
    entry.name = name;
  }
  if (changes.color !== undefined) {
    if (!validColor(changes.color)) throw new Error("颜色格式不正确");
    entry.color = changes.color;
  }
}

export function categoryCount(state, kind, id) {
  return allTasks(state).filter((task) => kind === "projects" ? task.projectId === id : task.tagIds.includes(id)).length;
}

export function removeCategory(state, kind, id, deleteTasks = false) {
  state[kind] = state[kind].filter((entry) => entry.id !== id);
  const transform = (list) => list.filter((task) => !(kind === "projects" && deleteTasks && task.projectId === id)).map((task) => {
    if (kind === "projects" && task.projectId === id) return { ...task, projectId: null };
    if (kind === "tags") return { ...task, tagIds: task.tagIds.filter((tagId) => tagId !== id) };
    return task;
  });
  for (const day of DAY_KEYS) state.itemsByDay[day] = transform(state.itemsByDay[day]);
  state.recurringItems = transform(state.recurringItems);
}

// Tokens start at a word boundary. Quotes allow whitespace and escaped quotes.
// Unquoted paths, URL fragments, C#, arithmetic and incomplete syntax remain text.
export function scanTokens(text) {
  const tokens = [];
  for (let i = 0; i < text.length; i++) {
    if (!/[#+]/.test(text[i]) || (i > 0 && !/\s/.test(text[i - 1]))) continue;
    const start = i;
    const kind = text[i] === "#" ? "tags" : "projects";
    let end = i + 1, name = "";
    const quoted = text[end] === '"';
    if (quoted) {
      end++;
      let closed = false;
      while (end < text.length) {
        if (text[end] === "\\" && /["\\]/.test(text[end + 1] ?? "")) { name += text[end + 1]; end += 2; }
        else if (text[end] === '"') { end++; closed = true; break; }
        else name += text[end++];
      }
      if (!closed || (end < text.length && !/\s/.test(text[end]))) continue;
    } else {
      while (end < text.length && !/\s/.test(text[end])) name += text[end++];
      if (!name || /[#+/\\:"<>]/.test(name) || (kind === 'projects' && /^\d+(?:\.\d+)?$/.test(name))) continue;
    }
    name = cleanName(name);
    if (name) tokens.push({ start, end, kind, name });
    i = end - 1;
  }
  return tokens;
}

export function parseClassification(text) {
  const tokens = scanTokens(text);
  let title = "", from = 0;
  for (const token of tokens) { title += text.slice(from, token.start); from = token.end; }
  title += text.slice(from);
  return { title: title.replace(/\s+/g, " ").trim(), tokens };
}

export function resolveClassification(state, tokens) {
  const project = tokens.filter((token) => token.kind === "projects").at(-1);
  return {
    projectId: project ? ensureCategory(state, "projects", project.name).id : null,
    tagIds: [...new Set(tokens.filter((token) => token.kind === "tags").map((token) => ensureCategory(state, "tags", token.name).id))],
  };
}

export function categoryToken(kind, name) {
  const quoted = /[\s#+/\\:"<>]/.test(name) || /^\d+(?:\.\d+)?$/.test(name);
  return (kind === "tags" ? "#" : "+") + (quoted ? JSON.stringify(name) : name);
}

export function editText(state, task) {
  const project = state.projects.find((entry) => entry.id === task.projectId);
  const tags = task.tagIds.map((id) => state.tags.find((entry) => entry.id === id)).filter(Boolean);
  return [task.title, ...(project ? [categoryToken("projects", project.name)] : []), ...tags.map((entry) => categoryToken("tags", entry.name))].join(" ");
}

export function activeToken(text, caret) {
  // Walk tokens up to the cursor, respecting quoted names containing spaces.
  let start = -1, quoted = false, escaped = false;
  for (let i = 0; i < caret; i++) {
    const c = text[i];
    if (escaped) { escaped = false; continue; }
    if (quoted && c === "\\") { escaped = true; continue; }
    if (start >= 0 && c === '"') { quoted = !quoted; continue; }
    if (!quoted && /\s/.test(c)) start = -1;
    if (!quoted && /[#+]/.test(c) && (i === 0 || /\s/.test(text[i - 1]))) start = i;
  }
  if (start < 0) return null;
  let query = text.slice(start + 1, caret);
  if (query.startsWith('"')) query = query.slice(1).replace(/"$/, "").replace(/\\(["\\])/g, "$1");
  else if (/[#+/\\:"<>]/.test(query) || (text[start] === '+' && /^\d+(?:\.\d+)?$/.test(query))) return null;
  let end = caret;
  while (end < text.length && (quoted || !/\s/.test(text[end]))) {
    if (text[end] === '"') quoted = false;
    end++;
  }
  return { start, end, query, kind: text[start] === "#" ? "tags" : "projects" };
}

export function suggestions(state, active) {
  const query = nameKey(active.query);
  const matches = state[active.kind].filter((entry) => nameKey(entry.name).includes(query))
    .sort((a, b) => Number(nameKey(b.name).startsWith(query)) - Number(nameKey(a.name).startsWith(query)) || a.name.localeCompare(b.name, "zh-CN"));
  const result = matches.slice(0, 7);
  if (query && !state[active.kind].some((entry) => nameKey(entry.name) === query)) {
    result.push({ name: cleanName(active.query), color: PALETTE[state.colorCounters[active.kind] % PALETTE.length], create: true });
  }
  return result;
}

export function matchesFilters(task, filters) {
  return (!filters.project || (filters.project === "none" ? !task.projectId : task.projectId === filters.project)) &&
    (!filters.tags.length || filters.tags.some((id) => task.tagIds.includes(id)));
}
