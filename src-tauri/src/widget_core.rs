use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DAY_KEYS: [&str; 3] = ["today", "tomorrow", "dayAfterTomorrow"];

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetTask {
    pub id: String,
    pub title: String,
    pub time: Option<String>,
    pub completed: bool,
    pub kind: String,
    pub day: Option<String>,
    pub scheduled_date: Option<String>,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
    pub project_color: Option<String>,
    pub tag_ids: Vec<String>,
    pub streak: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WidgetSnapshot {
    pub today: String,
    pub local_generation: u64,
    pub incomplete_count: usize,
    pub completed_count: usize,
    pub total_count: usize,
    pub today_tasks: Vec<WidgetTask>,
    pub recent_tasks: Vec<WidgetTask>,
    pub daily_tasks: Vec<WidgetTask>,
}

#[derive(Debug, Clone)]
struct Revision {
    revision_id: String,
    entity_type: String,
    entity_id: String,
    parent_revision_ids: Vec<String>,
    device_id: String,
    modified_at: i64,
    logical_counter: i64,
    deleted: bool,
    payload: Value,
}

impl Revision {
    fn from_value(v: &Value) -> Option<Self> {
        Some(Self {
            revision_id: v.get("revisionId")?.as_str()?.to_string(),
            entity_type: v.get("entityType")?.as_str()?.to_string(),
            entity_id: v.get("entityId")?.as_str()?.to_string(),
            parent_revision_ids: v
                .get("parentRevisionIds")?
                .as_array()?
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect(),
            device_id: v.get("deviceId")?.as_str()?.to_string(),
            modified_at: v.get("modifiedAt")?.as_i64()?,
            logical_counter: v.get("logicalCounter")?.as_i64()?,
            deleted: v.get("deleted")?.as_bool()?,
            payload: v.get("payload")?.clone(),
        })
    }

    fn entity_key(&self) -> String {
        format!("{}:{}", self.entity_type, self.entity_id)
    }
}

fn compare_revision(a: &Revision, b: &Revision) -> std::cmp::Ordering {
    a.modified_at
        .cmp(&b.modified_at)
        .then_with(|| a.logical_counter.cmp(&b.logical_counter))
        .then_with(|| a.device_id.cmp(&b.device_id))
        .then_with(|| a.revision_id.cmp(&b.revision_id))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    let key_json = serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key_json}:{}", canonical_json(&map[k]))
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        _ => serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
    }
}

fn revision_state(r: &Revision) -> String {
    let payload = if r.entity_type == "projects" || r.entity_type == "tags" {
        let mut cloned = r.payload.clone();
        if let Some(obj) = cloned.as_object_mut() {
            obj.insert("id".to_string(), Value::Null);
        }
        cloned
    } else {
        r.payload.clone()
    };
    canonical_json(&json!([r.deleted, payload]))
}

/// Howard Hinnant's civil-date to days-since-1970-01-01 conversion.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = y - if m <= 2 { 1 } else { 0 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of `days_from_civil`.
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

fn parse_iso_days(date: &str) -> Result<i64, String> {
    let mut parts = date.split('-');
    let y: i64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("日期格式不正确：{date}"))?;
    let m: i64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("日期格式不正确：{date}"))?;
    let d: i64 = parts
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("日期格式不正确：{date}"))?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(format!("日期格式不正确：{date}"));
    }
    let days = days_from_civil(y, m, d);
    let (ry, rm, rd) = civil_from_days(days);
    if (ry, rm, rd) != (y, m, d) {
        return Err(format!("日期格式不正确：{date}"));
    }
    Ok(days)
}

fn format_iso_days(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

fn shift_date(date: &str, delta_days: i64) -> Result<String, String> {
    Ok(format_iso_days(parse_iso_days(date)? + delta_days))
}

fn days_between(from: &str, to: &str) -> Result<i64, String> {
    Ok(parse_iso_days(to)? - parse_iso_days(from)?)
}

fn union_root(parent: &HashMap<String, String>, key: &str) -> String {
    let mut cur = key.to_string();
    while let Some(next) = parent.get(&cur) {
        cur = next.clone();
    }
    cur
}

fn category_aliases(revisions: &[Revision]) -> HashMap<String, String> {
    let mut parent: HashMap<String, String> = HashMap::new();
    let mut names: HashMap<String, String> = HashMap::new();
    for r in revisions {
        if r.entity_type != "projects" && r.entity_type != "tags" {
            continue;
        }
        let key = r.entity_key();
        let raw_name = r
            .payload
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        let name_key = format!("{}:{}", r.entity_type, raw_name);
        if let Some(existing) = names.get(&name_key) {
            let a = union_root(&parent, &key);
            let b = union_root(&parent, existing);
            if a != b {
                if a < b {
                    parent.insert(b, a);
                } else {
                    parent.insert(a, b);
                }
            }
        } else {
            names.insert(name_key, key);
        }
    }
    let mut aliases = HashMap::new();
    for r in revisions {
        if r.entity_type == "projects" || r.entity_type == "tags" {
            let key = r.entity_key();
            let root = union_root(&parent, &key);
            aliases.insert(key, root);
        }
    }
    aliases
}

fn entity_heads(revisions: &[Revision]) -> HashMap<String, Vec<Revision>> {
    let aliases = category_aliases(revisions);
    let key_of = |r: &Revision| -> String {
        let key = r.entity_key();
        aliases.get(&key).cloned().unwrap_or(key)
    };
    let by_id: HashMap<&str, &Revision> = revisions
        .iter()
        .map(|r| (r.revision_id.as_str(), r))
        .collect();
    let mut dependencies: HashMap<&str, usize> = HashMap::with_capacity(revisions.len());
    let mut children: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut ready: VecDeque<&str> = VecDeque::new();

    for r in revisions {
        let mut missing = 0usize;
        let r_key = key_of(r);
        for parent_id in &r.parent_revision_ids {
            missing += 1;
            if let Some(parent_rev) = by_id.get(parent_id.as_str()) {
                if key_of(parent_rev) == r_key {
                    children
                        .entry(parent_id.as_str())
                        .or_default()
                        .push(r.revision_id.as_str());
                }
            }
        }
        dependencies.insert(r.revision_id.as_str(), missing);
        if missing == 0 {
            ready.push_back(r.revision_id.as_str());
        }
    }

    let mut valid: HashSet<&str> = HashSet::new();
    while let Some(id) = ready.pop_front() {
        valid.insert(id);
        if let Some(kids) = children.get(id) {
            for &child in kids {
                if let Some(dep) = dependencies.get_mut(child) {
                    *dep -= 1;
                    if *dep == 0 {
                        ready.push_back(child);
                    }
                }
            }
        }
    }

    let mut parents: HashSet<&str> = HashSet::new();
    for r in revisions {
        if valid.contains(r.revision_id.as_str()) {
            for p in &r.parent_revision_ids {
                parents.insert(p.as_str());
            }
        }
    }

    let mut groups: HashMap<String, Vec<Revision>> = HashMap::new();
    for r in revisions {
        if valid.contains(r.revision_id.as_str()) && !parents.contains(r.revision_id.as_str()) {
            groups.entry(key_of(r)).or_default().push(r.clone());
        }
    }
    for list in groups.values_mut() {
        list.sort_by(compare_revision);
    }
    groups
}

fn current_entities(revisions: &[Revision]) -> HashMap<String, Revision> {
    entity_heads(revisions)
        .into_iter()
        .filter_map(|(k, mut heads)| heads.pop().map(|winner| (k, winner)))
        .collect()
}

pub fn project_widget_snapshot(doc: &Value, today: &str) -> Result<WidgetSnapshot, String> {
    let today_days = parse_iso_days(today)?;
    let today_norm = format_iso_days(today_days);
    let local_generation = doc
        .get("localGeneration")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let raw_revisions = doc
        .get("revisions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "缺少版本记录".to_string())?;
    let revisions: Vec<Revision> = raw_revisions
        .iter()
        .filter_map(Revision::from_value)
        .collect();
    let current = current_entities(&revisions);
    let identity_aliases = category_aliases(&revisions);
    let mut id_aliases: HashMap<String, String> = identity_aliases
        .into_iter()
        .map(|(k, v)| {
            let id = v.split_once(':').map(|(_, id)| id).unwrap_or(&v).to_string();
            (k, id)
        })
        .collect();

    let mut live_projects: HashMap<String, (String, String)> = HashMap::new();
    let mut live_tags: HashSet<String> = HashSet::new();

    for kind in ["projects", "tags"] {
        let mut parent: HashMap<String, String> = HashMap::new();
        let mut names: HashMap<String, String> = HashMap::new();
        let resolve = |id: &str,
                       id_aliases: &HashMap<String, String>,
                       parent: &HashMap<String, String>|
         -> String {
            let mut cur = id_aliases
                .get(&format!("{kind}:{id}"))
                .cloned()
                .unwrap_or_else(|| id.to_string());
            while let Some(next) = parent.get(&cur) {
                cur = next.clone();
            }
            cur
        };

        for r in revisions.iter().filter(|r| r.entity_type == kind) {
            let name = r
                .payload
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_lowercase();
            if let Some(existing_id) = names.get(&name) {
                let a = resolve(&r.entity_id, &id_aliases, &parent);
                let b = resolve(existing_id, &id_aliases, &parent);
                if a != b {
                    if a < b {
                        parent.insert(b, a);
                    } else {
                        parent.insert(a, b);
                    }
                }
            } else {
                names.insert(name, r.entity_id.clone());
            }
        }

        let mut groups: HashMap<String, Vec<&Revision>> = HashMap::new();
        for r in current.values().filter(|r| r.entity_type == kind) {
            let id = resolve(&r.entity_id, &id_aliases, &parent);
            id_aliases.insert(format!("{kind}:{}", r.entity_id), id.clone());
            groups.entry(id).or_default().push(r);
        }
        for (id, mut list) in groups {
            list.sort_by(|a, b| compare_revision(a, b));
            if let Some(winner) = list.last() {
                if !winner.deleted {
                    if kind == "projects" {
                        let name = winner
                            .payload
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let color = winner
                            .payload
                            .get("color")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        live_projects.insert(id, (name, color));
                    } else {
                        live_tags.insert(id);
                    }
                }
            }
        }
    }

    let completions: HashMap<String, bool> = current
        .values()
        .filter(|r| r.entity_type == "completion" && !r.deleted)
        .filter_map(|r| {
            let done = r.payload.get("completed")?.as_bool()?;
            Some((r.entity_id.clone(), done))
        })
        .collect();

    let orders: HashMap<String, f64> = current
        .values()
        .filter(|r| r.entity_type == "order" && !r.deleted)
        .filter_map(|r| {
            let pos = r.payload.get("position")?.as_f64()?;
            Some((r.entity_id.clone(), pos))
        })
        .collect();

    let mut by_day: [Vec<(WidgetTask, f64)>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut daily_tasks: Vec<(WidgetTask, f64)> = Vec::new();

    for r in current.values() {
        if r.entity_type != "task" || r.deleted {
            continue;
        }
        let p = &r.payload;
        let Some(id) = p.get("id").and_then(|v| v.as_str()).map(String::from) else {
            continue;
        };
        let title = p
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let kind = p
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("one_off")
            .to_string();
        let time = p.get("time").and_then(|v| v.as_str()).map(String::from);
        let created_at = p.get("createdAt").and_then(|v| v.as_f64()).unwrap_or(0.0);

        let raw_project = p.get("projectId").and_then(|v| v.as_str());
        let resolved_project = raw_project.map(|pid| {
            id_aliases
                .get(&format!("projects:{pid}"))
                .cloned()
                .unwrap_or_else(|| pid.to_string())
        });
        let (project_id, project_name, project_color) = match resolved_project {
            Some(pid) => match live_projects.get(&pid) {
                Some((name, color)) => (Some(pid), Some(name.clone()), Some(color.clone())),
                None => (None, None, None),
            },
            None => (None, None, None),
        };

        let mut seen_tags: HashSet<String> = HashSet::new();
        let mut tag_ids: Vec<String> = Vec::new();
        if let Some(arr) = p.get("tagIds").and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(tid) = item.as_str() {
                    let resolved = id_aliases
                        .get(&format!("tags:{tid}"))
                        .cloned()
                        .unwrap_or_else(|| tid.to_string());
                    if live_tags.contains(&resolved) && seen_tags.insert(resolved.clone()) {
                        tag_ids.push(resolved);
                    }
                }
            }
        }

        if kind == "daily" {
            let completed = *completions
                .get(&format!("{id}/{today_norm}"))
                .unwrap_or(&false);
            let mut day = if completed {
                today_norm.clone()
            } else {
                shift_date(&today_norm, -1)?
            };
            let mut streak: u32 = 0;
            while *completions.get(&format!("{id}/{day}")).unwrap_or(&false) {
                streak += 1;
                day = shift_date(&day, -1)?;
            }
            if let Some(legacy) = p.get("legacyStreak").and_then(|v| v.as_object()) {
                if !completions.contains_key(&format!("{id}/{day}")) {
                    if let (Some(l_date), Some(l_count)) = (
                        legacy.get("date").and_then(|v| v.as_str()),
                        legacy.get("count").and_then(|v| v.as_i64()),
                    ) {
                        if let Ok(start) = shift_date(l_date, 1 - l_count) {
                            if day.as_str() >= start.as_str() && day.as_str() <= l_date {
                                let prefix = format!("{id}/");
                                let mut cancelled: Option<String> = None;
                                for (k, done) in &completions {
                                    if !done && k.starts_with(&prefix) {
                                        let d_part = &k[prefix.len()..];
                                        if d_part >= start.as_str() && d_part <= day.as_str() {
                                            if cancelled
                                                .as_deref()
                                                .map_or(true, |prev| d_part > prev)
                                            {
                                                cancelled = Some(d_part.to_string());
                                            }
                                        }
                                    }
                                }
                                let from_date = cancelled.as_deref().unwrap_or(start.as_str());
                                if let Ok(diff) = days_between(from_date, &day) {
                                    let extra = diff + if cancelled.is_some() { 0 } else { 1 };
                                    if extra > 0 {
                                        streak += extra as u32;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            daily_tasks.push((
                WidgetTask {
                    id,
                    title,
                    time,
                    completed,
                    kind,
                    day: None,
                    scheduled_date: None,
                    project_id,
                    project_name,
                    project_color,
                    tag_ids,
                    streak,
                },
                created_at,
            ));
        } else {
            let completed = p.get("completed").and_then(|v| v.as_bool()).unwrap_or(false);
            let Some(scheduled_date) = p
                .get("scheduledDate")
                .and_then(|v| v.as_str())
                .map(String::from)
            else {
                continue;
            };
            let Ok(sched_days) = parse_iso_days(&scheduled_date) else {
                continue;
            };
            let offset = sched_days - today_days;
            if offset < 0 && completed {
                continue;
            }
            if offset <= 2 {
                let bucket = offset.max(0) as usize;
                by_day[bucket].push((
                    WidgetTask {
                        id,
                        title,
                        time,
                        completed,
                        kind,
                        day: Some(DAY_KEYS[bucket].to_string()),
                        scheduled_date: Some(scheduled_date),
                        project_id,
                        project_name,
                        project_color,
                        tag_ids,
                        streak: 0,
                    },
                    created_at,
                ));
            }
        }
    }

    let sort_tasks = |list: &mut Vec<(WidgetTask, f64)>| {
        list.sort_by(|(a, a_created), (b, b_created)| {
            let a_pos = orders.get(&a.id).copied().unwrap_or(-*a_created);
            let b_pos = orders.get(&b.id).copied().unwrap_or(-*b_created);
            a.completed
                .cmp(&b.completed)
                .then_with(|| {
                    a_pos
                        .partial_cmp(&b_pos)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    b_created
                        .partial_cmp(a_created)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.id.cmp(&b.id))
        });
    };

    for bucket in &mut by_day {
        sort_tasks(bucket);
    }
    sort_tasks(&mut daily_tasks);

    let mut recent_tasks = Vec::new();
    'recent: for bucket in &by_day {
        for (task, _) in bucket {
            if !task.completed {
                recent_tasks.push(task.clone());
                if recent_tasks.len() >= 3 {
                    break 'recent;
                }
            }
        }
    }

    let today_tasks: Vec<WidgetTask> = by_day[0].iter().map(|(t, _)| t.clone()).collect();
    let daily_tasks: Vec<WidgetTask> = daily_tasks.into_iter().map(|(t, _)| t).collect();
    let incomplete_count = today_tasks.iter().filter(|t| !t.completed).count();
    let completed_count = today_tasks.iter().filter(|t| t.completed).count();
    let total_count = today_tasks.len();

    Ok(WidgetSnapshot {
        today: today_norm,
        local_generation,
        incomplete_count,
        completed_count,
        total_count,
        today_tasks,
        recent_tasks,
        daily_tasks,
    })
}

fn open_db(db_path: &Path) -> Result<Connection, String> {
    let db = Connection::open(db_path).map_err(|e| e.to_string())?;
    db.busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")
        .map_err(|e| e.to_string())?;
    Ok(db)
}

pub fn widget_load_snapshot(db_path: &Path, today: &str) -> Result<WidgetSnapshot, String> {
    if !db_path.exists() {
        return Ok(WidgetSnapshot {
            today: format_iso_days(parse_iso_days(today)?),
            local_generation: 0,
            incomplete_count: 0,
            completed_count: 0,
            total_count: 0,
            today_tasks: Vec::new(),
            recent_tasks: Vec::new(),
            daily_tasks: Vec::new(),
        });
    }
    let db = open_db(db_path)?;
    let raw: Option<String> = db
        .query_row("SELECT json FROM document WHERE id=1", [], |row| row.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(raw) = raw else {
        return Ok(WidgetSnapshot {
            today: format_iso_days(parse_iso_days(today)?),
            local_generation: 0,
            incomplete_count: 0,
            completed_count: 0,
            total_count: 0,
            today_tasks: Vec::new(),
            recent_tasks: Vec::new(),
            daily_tasks: Vec::new(),
        });
    };
    let doc: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    project_widget_snapshot(&doc, today)
}

fn generate_uuid_v4() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    static SEQ: AtomicU64 = AtomicU64::new(1);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let r1 = RandomState::new().build_hasher().finish();
    let r2 = RandomState::new().build_hasher().finish();

    let mut hasher = Sha256::new();
    hasher.update(now.to_le_bytes());
    hasher.update(seq.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(r1.to_le_bytes());
    hasher.update(r2.to_le_bytes());
    let digest = hasher.finalize();

    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

pub fn widget_toggle_task(
    db_path: &Path,
    task_id: &str,
    today: &str,
) -> Result<WidgetSnapshot, String> {
    let today_norm = format_iso_days(parse_iso_days(today)?);
    let mut db = open_db(db_path)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let raw: String = tx
        .query_row("SELECT json FROM document WHERE id=1", [], |row| row.get(0))
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "本地数据库尚未初始化".to_string())?;
    let mut doc: Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    if doc["schemaVersion"] != 3 {
        return Err("不支持的数据版本".into());
    }

    let device_id = doc
        .get("deviceId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "缺少设备 ID".to_string())?
        .to_string();
    let revisions: Vec<Revision> = doc
        .get("revisions")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "缺少版本记录".to_string())?
        .iter()
        .filter_map(Revision::from_value)
        .collect();

    let heads_map = entity_heads(&revisions);
    let task_key = format!("task:{task_id}");
    let task_heads = heads_map
        .get(&task_key)
        .ok_or_else(|| format!("未找到任务：{task_id}"))?;
    let task_winner = task_heads
        .last()
        .ok_or_else(|| format!("未找到任务：{task_id}"))?;
    if task_winner.deleted {
        return Err("任务已删除".into());
    }

    let kind = task_winner
        .payload
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("one_off");

    let clock_time = doc
        .get("clock")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let clock_counter = doc
        .get("clock")
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(1))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let time = now_ms.max(clock_time);
    let counter = if time == clock_time {
        clock_counter + 1
    } else {
        0
    };
    doc["clock"] = json!([time, counter]);

    let revision_id = generate_uuid_v4();
    let new_rev = if kind == "daily" {
        let comp_id = format!("{task_id}/{today_norm}");
        let comp_key = format!("completion:{comp_id}");
        let comp_heads = heads_map.get(&comp_key).cloned().unwrap_or_default();
        let prev_winner = comp_heads.last();
        let prev_completed = prev_winner
            .filter(|r| !r.deleted)
            .and_then(|r| r.payload.get("completed").and_then(|v| v.as_bool()))
            .unwrap_or(false);
        let parent_ids: Vec<String> = match prev_winner {
            Some(winner) => {
                let target_state = revision_state(winner);
                comp_heads
                    .iter()
                    .filter(|r| revision_state(r) == target_state)
                    .map(|r| r.revision_id.clone())
                    .collect()
            }
            None => Vec::new(),
        };
        json!({
            "revisionId": revision_id,
            "entityType": "completion",
            "entityId": comp_id,
            "parentRevisionIds": parent_ids,
            "deviceId": device_id,
            "modifiedAt": time,
            "logicalCounter": counter,
            "deleted": false,
            "payload": {
                "taskId": task_id,
                "date": today_norm,
                "completed": !prev_completed
            }
        })
    } else {
        let prev_completed = task_winner
            .payload
            .get("completed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut payload = task_winner.payload.clone();
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("completed".to_string(), Value::Bool(!prev_completed));
        }
        let target_state = revision_state(task_winner);
        let parent_ids: Vec<String> = task_heads
            .iter()
            .filter(|r| revision_state(r) == target_state)
            .map(|r| r.revision_id.clone())
            .collect();
        json!({
            "revisionId": revision_id,
            "entityType": "task",
            "entityId": task_id,
            "parentRevisionIds": parent_ids,
            "deviceId": device_id,
            "modifiedAt": time,
            "logicalCounter": counter,
            "deleted": false,
            "payload": payload
        })
    };

    let entity_key = format!(
        "{}:{}",
        new_rev["entityType"].as_str().unwrap_or(""),
        new_rev["entityId"].as_str().unwrap_or("")
    );

    doc.get_mut("revisions")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| "缺少版本记录".to_string())?
        .push(new_rev.clone());
    doc.get_mut("outbox")
        .and_then(|v| v.as_array_mut())
        .ok_or_else(|| "缺少上传队列".to_string())?
        .push(Value::String(revision_id.clone()));

    let next_generation = doc
        .get("localGeneration")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;
    doc["localGeneration"] = json!(next_generation);

    let updated_json = doc.to_string();
    tx.execute(
        "INSERT INTO document(id,json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
        [&updated_json],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR IGNORE INTO revisions(id,entity_key,json) VALUES(?1,?2,?3)",
        params![revision_id, entity_key, new_rev.to_string()],
    )
    .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT OR IGNORE INTO outbox(revision_id) VALUES(?1)",
        [&revision_id],
    )
    .map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())?;
    project_widget_snapshot(&doc, &today_norm)
}

#[tauri::command]
pub fn widget_snapshot(
    app: tauri::AppHandle,
    today: String,
) -> Result<WidgetSnapshot, String> {
    let path = crate::sync_storage::storage_db_path(&app)?;
    widget_load_snapshot(&path, &today)
}

#[tauri::command]
pub fn widget_toggle(
    app: tauri::AppHandle,
    task_id: String,
    today: String,
) -> Result<WidgetSnapshot, String> {
    let path = crate::sync_storage::storage_db_path(&app)?;
    let snapshot = widget_toggle_task(&path, &task_id, &today)?;
    crate::sync_storage::notify_widgets(&app);
    Ok(snapshot)
}

fn c_json_response<T: Serialize>(result: Result<T, String>) -> *mut c_char {
    let body = match result {
        Ok(data) => json!({ "ok": true, "snapshot": data }).to_string(),
        Err(error) => json!({ "ok": false, "error": error }).to_string(),
    };
    CString::new(body)
        .unwrap_or_else(|_| CString::new("{\"ok\":false,\"error\":\"utf8\"}").unwrap())
        .into_raw()
}

/// C-ABI entry point for iOS WidgetKit / Android JNI to load a widget snapshot without WebView.
#[no_mangle]
pub unsafe extern "C" fn floating_todo_widget_snapshot(
    db_path: *const c_char,
    today: *const c_char,
) -> *mut c_char {
    if db_path.is_null() || today.is_null() {
        return c_json_response::<WidgetSnapshot>(Err("invalid null argument".into()));
    }
    let Ok(path_str) = CStr::from_ptr(db_path).to_str() else {
        return c_json_response::<WidgetSnapshot>(Err("invalid db_path utf-8".into()));
    };
    let Ok(today_str) = CStr::from_ptr(today).to_str() else {
        return c_json_response::<WidgetSnapshot>(Err("invalid today utf-8".into()));
    };
    c_json_response(widget_load_snapshot(Path::new(path_str), today_str))
}

/// C-ABI entry point for iOS WidgetKit AppIntent / Android PendingIntent to toggle a task atomically.
#[no_mangle]
pub unsafe extern "C" fn floating_todo_widget_toggle(
    db_path: *const c_char,
    task_id: *const c_char,
    today: *const c_char,
) -> *mut c_char {
    if db_path.is_null() || task_id.is_null() || today.is_null() {
        return c_json_response::<WidgetSnapshot>(Err("invalid null argument".into()));
    }
    let Ok(path_str) = CStr::from_ptr(db_path).to_str() else {
        return c_json_response::<WidgetSnapshot>(Err("invalid db_path utf-8".into()));
    };
    let Ok(task_str) = CStr::from_ptr(task_id).to_str() else {
        return c_json_response::<WidgetSnapshot>(Err("invalid task_id utf-8".into()));
    };
    let Ok(today_str) = CStr::from_ptr(today).to_str() else {
        return c_json_response::<WidgetSnapshot>(Err("invalid today utf-8".into()));
    };
    c_json_response(widget_toggle_task(Path::new(path_str), task_str, today_str))
}

/// Releases a string returned by `floating_todo_widget_snapshot` or `floating_todo_widget_toggle`.
#[no_mangle]
pub unsafe extern "C" fn floating_todo_widget_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        let _ = CString::from_raw(ptr);
    }
}

#[cfg(target_os = "android")]
mod android_jni {
    use super::*;
    use jni::objects::{JClass, JString};
    use jni::sys::jstring;
    use jni::JNIEnv;

    fn jni_json_response<T: Serialize>(env: JNIEnv, result: Result<T, String>) -> jstring {
        let body = match result {
            Ok(data) => json!({ "ok": true, "snapshot": data }).to_string(),
            Err(error) => json!({ "ok": false, "error": error }).to_string(),
        };
        env.new_string(body)
            .map(|s| s.into_raw())
            .unwrap_or(std::ptr::null_mut())
    }

    #[no_mangle]
    pub extern "system" fn Java_com_gaopengfei_floating_1todo_WidgetBridge_nativeLoadSnapshot(
        mut env: JNIEnv,
        _class: JClass,
        db_path: JString,
        today: JString,
    ) -> jstring {
        let path_str = match env.get_string(&db_path) {
            Ok(s) => s.to_string_lossy().into_owned(),
            Err(e) => return jni_json_response::<WidgetSnapshot>(env, Err(e.to_string())),
        };
        let today_str = match env.get_string(&today) {
            Ok(s) => s.to_string_lossy().into_owned(),
            Err(e) => return jni_json_response::<WidgetSnapshot>(env, Err(e.to_string())),
        };
        let res = widget_load_snapshot(Path::new(&path_str), &today_str);
        jni_json_response(env, res)
    }

    #[no_mangle]
    pub extern "system" fn Java_com_gaopengfei_floating_1todo_WidgetBridge_nativeToggleTask(
        mut env: JNIEnv,
        _class: JClass,
        db_path: JString,
        task_id: JString,
        today: JString,
    ) -> jstring {
        let path_str = match env.get_string(&db_path) {
            Ok(s) => s.to_string_lossy().into_owned(),
            Err(e) => return jni_json_response::<WidgetSnapshot>(env, Err(e.to_string())),
        };
        let task_str = match env.get_string(&task_id) {
            Ok(s) => s.to_string_lossy().into_owned(),
            Err(e) => return jni_json_response::<WidgetSnapshot>(env, Err(e.to_string())),
        };
        let today_str = match env.get_string(&today) {
            Ok(s) => s.to_string_lossy().into_owned(),
            Err(e) => return jni_json_response::<WidgetSnapshot>(env, Err(e.to_string())),
        };
        let res = widget_toggle_task(Path::new(&path_str), &task_str, &today_str);
        jni_json_response(env, res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "floating-todo-widget-test-{}-{}",
            std::process::id(),
            generate_uuid_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    fn seed_db(path: &Path, doc: &Value) {
        let db = open_db(path).unwrap();
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS document (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS revisions (id TEXT PRIMARY KEY, entity_key TEXT NOT NULL, json TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS outbox (revision_id TEXT PRIMARY KEY);",
        )
        .unwrap();
        db.execute(
            "INSERT INTO document(id,json) VALUES(1,?1)",
            [doc.to_string()],
        )
        .unwrap();
    }

    #[test]
    fn projects_rollover_order_categories_and_daily_streaks() {
        let doc = json!({
            "schemaVersion": 3,
            "protocolVersion": 1,
            "localGeneration": 4,
            "deviceId": "dev-a",
            "clock": [1000, 5],
            "outbox": [],
            "revisions": [
                {
                    "revisionId": "p1", "entityType": "projects", "entityId": "proj-1",
                    "parentRevisionIds": [], "deviceId": "dev-a", "modifiedAt": 100, "logicalCounter": 0, "deleted": false,
                    "payload": { "id": "proj-1", "name": "工作", "color": "#2563eb" }
                },
                {
                    "revisionId": "t-overdue-open", "entityType": "task", "entityId": "task-1",
                    "parentRevisionIds": [], "deviceId": "dev-a", "modifiedAt": 101, "logicalCounter": 0, "deleted": false,
                    "payload": { "id": "task-1", "kind": "one_off", "title": "过期未完成", "detail": "", "time": "09:30", "completed": false, "createdAt": 10, "scheduledDate": "2026-11-21", "projectId": "proj-1", "tagIds": [] }
                },
                {
                    "revisionId": "t-overdue-done", "entityType": "task", "entityId": "task-2",
                    "parentRevisionIds": [], "deviceId": "dev-a", "modifiedAt": 102, "logicalCounter": 0, "deleted": false,
                    "payload": { "id": "task-2", "kind": "one_off", "title": "过期已完成", "detail": "", "time": null, "completed": true, "createdAt": 11, "scheduledDate": "2026-11-21", "projectId": null, "tagIds": [] }
                },
                {
                    "revisionId": "t-today", "entityType": "task", "entityId": "task-3",
                    "parentRevisionIds": [], "deviceId": "dev-a", "modifiedAt": 103, "logicalCounter": 0, "deleted": false,
                    "payload": { "id": "task-3", "kind": "one_off", "title": "今日优先", "detail": "", "time": "15:30", "completed": false, "createdAt": 12, "scheduledDate": "2026-11-23", "projectId": "proj-1", "tagIds": [] }
                },
                {
                    "revisionId": "t-tomorrow", "entityType": "task", "entityId": "task-4",
                    "parentRevisionIds": [], "deviceId": "dev-a", "modifiedAt": 104, "logicalCounter": 0, "deleted": false,
                    "payload": { "id": "task-4", "kind": "one_off", "title": "明日会议", "detail": "", "time": "10:00", "completed": false, "createdAt": 13, "scheduledDate": "2026-11-24", "projectId": null, "tagIds": [] }
                },
                {
                    "revisionId": "ord-3", "entityType": "order", "entityId": "task-3",
                    "parentRevisionIds": [], "deviceId": "dev-a", "modifiedAt": 105, "logicalCounter": 0, "deleted": false,
                    "payload": { "position": -5.0 }
                },
                {
                    "revisionId": "ord-1", "entityType": "order", "entityId": "task-1",
                    "parentRevisionIds": [], "deviceId": "dev-a", "modifiedAt": 106, "logicalCounter": 0, "deleted": false,
                    "payload": { "position": 1.0 }
                },
                {
                    "revisionId": "d1", "entityType": "task", "entityId": "habit-1",
                    "parentRevisionIds": [], "deviceId": "dev-a", "modifiedAt": 107, "logicalCounter": 0, "deleted": false,
                    "payload": { "id": "habit-1", "kind": "daily", "title": "晨跑", "detail": "", "time": "07:00", "completed": false, "createdAt": 14, "scheduledDate": null, "projectId": null, "tagIds": [], "legacyStreak": { "date": "2026-11-21", "count": 5 } }
                },
                {
                    "revisionId": "c1", "entityType": "completion", "entityId": "habit-1/2026-11-22",
                    "parentRevisionIds": [], "deviceId": "dev-a", "modifiedAt": 108, "logicalCounter": 0, "deleted": false,
                    "payload": { "taskId": "habit-1", "date": "2026-11-22", "completed": true }
                }
            ]
        });

        let snap = project_widget_snapshot(&doc, "2026-11-23").unwrap();
        assert_eq!(snap.local_generation, 4);
        assert_eq!(snap.incomplete_count, 2);
        assert_eq!(snap.completed_count, 0);
        // task-2 (overdue completed) excluded; task-3 sorts before task-1 by order.position
        let today_ids: Vec<&str> = snap.today_tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(today_ids, vec!["task-3", "task-1"]);
        assert_eq!(snap.today_tasks[0].project_name.as_deref(), Some("工作"));
        assert_eq!(snap.today_tasks[0].project_color.as_deref(), Some("#2563eb"));

        // recentTasks picks today incomplete then tomorrow incomplete, capped at 3
        let recent_ids: Vec<&str> = snap.recent_tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(recent_ids, vec!["task-3", "task-1", "task-4"]);

        // daily task streak: 2026-11-22 (1) + legacyStreak ending 2026-11-21 (5) = 6
        assert_eq!(snap.daily_tasks.len(), 1);
        assert!(!snap.daily_tasks[0].completed);
        assert_eq!(snap.daily_tasks[0].streak, 6);
    }

    #[test]
    fn toggle_one_off_and_daily_tasks_writes_revisions_outbox_and_increments_generation() {
        let path = temp_db_path("todo-v3.sqlite3");
        let doc = json!({
            "schemaVersion": 3,
            "protocolVersion": 1,
            "localGeneration": 2,
            "deviceId": "dev-mobile",
            "clock": [500, 1],
            "outbox": [],
            "revisions": [
                {
                    "revisionId": "r-task", "entityType": "task", "entityId": "t1",
                    "parentRevisionIds": [], "deviceId": "dev-mobile", "modifiedAt": 500, "logicalCounter": 0, "deleted": false,
                    "payload": { "id": "t1", "kind": "one_off", "title": "提交周报", "detail": "", "time": "15:30", "completed": false, "createdAt": 100, "scheduledDate": "2026-11-23", "projectId": null, "tagIds": [] }
                },
                {
                    "revisionId": "r-daily", "entityType": "task", "entityId": "h1",
                    "parentRevisionIds": [], "deviceId": "dev-mobile", "modifiedAt": 500, "logicalCounter": 1, "deleted": false,
                    "payload": { "id": "h1", "kind": "daily", "title": "喝水", "detail": "", "time": null, "completed": false, "createdAt": 101, "scheduledDate": null, "projectId": null, "tagIds": [] }
                }
            ]
        });
        seed_db(&path, &doc);

        // Toggle one_off task to completed
        let snap1 = widget_toggle_task(&path, "t1", "2026-11-23").unwrap();
        assert_eq!(snap1.local_generation, 3);
        assert_eq!(snap1.incomplete_count, 0);
        assert_eq!(snap1.completed_count, 1);
        assert!(snap1.today_tasks[0].completed);

        // Toggle daily task to completed
        let snap2 = widget_toggle_task(&path, "h1", "2026-11-23").unwrap();
        assert_eq!(snap2.local_generation, 4);
        assert!(snap2.daily_tasks[0].completed);
        assert_eq!(snap2.daily_tasks[0].streak, 1);

        // Toggle daily task back to incomplete
        let snap3 = widget_toggle_task(&path, "h1", "2026-11-23").unwrap();
        assert_eq!(snap3.local_generation, 5);
        assert!(!snap3.daily_tasks[0].completed);
        assert_eq!(snap3.daily_tasks[0].streak, 0);

        // Verify outbox and revisions tables have all 3 widget-created revisions
        let db = open_db(&path).unwrap();
        let outbox_count: i64 = db
            .query_row("SELECT COUNT(*) FROM outbox", [], |r| r.get(0))
            .unwrap();
        assert_eq!(outbox_count, 3);

        // Verify C-ABI roundtrip
        unsafe {
            let c_path = CString::new(path.to_str().unwrap()).unwrap();
            let c_today = CString::new("2026-11-23").unwrap();
            let ptr = floating_todo_widget_snapshot(c_path.as_ptr(), c_today.as_ptr());
            let json_str = CStr::from_ptr(ptr).to_str().unwrap().to_string();
            floating_todo_widget_free_string(ptr);
            let parsed: Value = serde_json::from_str(&json_str).unwrap();
            assert_eq!(parsed["ok"], true);
            assert_eq!(parsed["snapshot"]["localGeneration"], 5);
        }

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
