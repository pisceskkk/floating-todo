import Foundation
import SQLite3

struct WidgetTaskItem: Identifiable, Hashable {
    let id: String
    let title: String
    let time: String?
    let completed: Bool
    let kind: String
    let projectName: String?
    let projectColor: String?
    let streak: Int
}

struct WidgetSnapshotData {
    let today: String
    let incompleteCount: Int
    let completedCount: Int
    let todayTasks: [WidgetTaskItem]
    let recentTasks: [WidgetTaskItem]
    let dailyTasks: [WidgetTaskItem]

    static func empty(today: String) -> WidgetSnapshotData {
        WidgetSnapshotData(
            today: today,
            incompleteCount: 0,
            completedCount: 0,
            todayTasks: [],
            recentTasks: [],
            dailyTasks: []
        )
    }
}

private struct RawRevision {
    let revisionId: String
    let entityType: String
    let entityId: String
    let parentRevisionIds: [String]
    let deviceId: String
    let modifiedAt: Int64
    let logicalCounter: Int64
    let deleted: Bool
    let payload: [String: Any]

    var entityKey: String { "\(entityType):\(entityId)" }

    init?(dict: [String: Any]) {
        guard
            let revisionId = dict["revisionId"] as? String,
            let entityType = dict["entityType"] as? String,
            let entityId = dict["entityId"] as? String,
            let parentRevisionIds = dict["parentRevisionIds"] as? [String],
            let deviceId = dict["deviceId"] as? String,
            let modifiedAt = (dict["modifiedAt"] as? NSNumber)?.int64Value,
            let logicalCounter = (dict["logicalCounter"] as? NSNumber)?.int64Value,
            let deleted = dict["deleted"] as? Bool,
            let payload = dict["payload"] as? [String: Any]
        else {
            return nil
        }
        self.revisionId = revisionId
        self.entityType = entityType
        self.entityId = entityId
        self.parentRevisionIds = parentRevisionIds
        self.deviceId = deviceId
        self.modifiedAt = modifiedAt
        self.logicalCounter = logicalCounter
        self.deleted = deleted
        self.payload = payload
    }
}

enum TodoWidgetStore {
    private static let sqliteTransient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

    static func todayIso() -> String {
        let df = DateFormatter()
        df.locale = Locale(identifier: "en_US_POSIX")
        df.dateFormat = "yyyy-MM-dd"
        return df.string(from: Date())
    }

    static func sharedDatabasePath() -> String? {
        var candidates: [String] = []
        if let extBundleId = Bundle.main.bundleIdentifier, !extBundleId.isEmpty {
            // Strip extension suffix (e.g. com.gaopengfei.floating-todo.<TEAMID>.TodoWidget)
            // so SideStore/AltStore re-signed App Groups resolve automatically.
            let parts = extBundleId.split(separator: ".")
            if parts.count > 1 {
                let mainBundleId = parts.dropLast().joined(separator: ".")
                candidates.append("group.\(mainBundleId)")
            }
            candidates.append("group.\(extBundleId)")
        }
        if !candidates.contains("group.com.gaopengfei.floating-todo") {
            candidates.append("group.com.gaopengfei.floating-todo")
        }
        for groupId in candidates {
            if let url = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: groupId) {
                return url.appendingPathComponent("todo-v3.sqlite3").path
            }
        }
        return nil
    }

    static func loadSnapshot(today: String = todayIso()) -> WidgetSnapshotData {
        guard let dbPath = sharedDatabasePath(), FileManager.default.fileExists(atPath: dbPath) else {
            return .empty(today: today)
        }
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK, let db = db else {
            if let db = db { sqlite3_close(db) }
            return .empty(today: today)
        }
        defer { sqlite3_close(db) }
        sqlite3_busy_timeout(db, 5000)

        guard let rawJson = queryDocumentJson(db: db),
              let data = rawJson.data(using: .utf8),
              let doc = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else {
            return .empty(today: today)
        }
        return projectSnapshot(doc: doc, today: today)
    }

    @discardableResult
    static func toggleTask(taskId: String, today: String = todayIso()) -> WidgetSnapshotData {
        guard let dbPath = sharedDatabasePath(), FileManager.default.fileExists(atPath: dbPath) else {
            return .empty(today: today)
        }
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READWRITE, nil) == SQLITE_OK, let db = db else {
            if let db = db { sqlite3_close(db) }
            return .empty(today: today)
        }
        defer { sqlite3_close(db) }
        sqlite3_busy_timeout(db, 5000)
        sqlite3_exec(db, "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;", nil, nil, nil)

        guard sqlite3_exec(db, "BEGIN IMMEDIATE;", nil, nil, nil) == SQLITE_OK else {
            return loadSnapshot(today: today)
        }

        func rollback() {
            sqlite3_exec(db, "ROLLBACK;", nil, nil, nil)
        }

        guard let rawJson = queryDocumentJson(db: db),
              let data = rawJson.data(using: .utf8),
              var doc = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              (doc["schemaVersion"] as? NSNumber)?.intValue == 3,
              let deviceId = doc["deviceId"] as? String,
              var rawRevs = doc["revisions"] as? [[String: Any]],
              var outbox = doc["outbox"] as? [String]
        else {
            rollback()
            return .empty(today: today)
        }

        let revisions = rawRevs.compactMap(RawRevision.init)
        let headsMap = entityHeads(revisions)
        guard let taskHeads = headsMap["task:\(taskId)"],
              let taskWinner = taskHeads.last,
              !taskWinner.deleted
        else {
            rollback()
            return projectSnapshot(doc: doc, today: today)
        }

        let clockArr = doc["clock"] as? [NSNumber] ?? []
        let clockTime = clockArr.first?.int64Value ?? 0
        let clockCounter = (clockArr.count > 1 ? clockArr[1].int64Value : 0)
        let nowMs = Int64(Date().timeIntervalSince1970 * 1000)
        let time = max(nowMs, clockTime)
        let counter = (time == clockTime) ? (clockCounter + 1) : 0
        doc["clock"] = [time, counter]

        let revisionId = UUID().uuidString.lowercased()
        let kind = (taskWinner.payload["kind"] as? String) ?? "one_off"
        let newRev: [String: Any]
        let entityKey: String

        if kind == "daily" {
            let compId = "\(taskId)/\(today)"
            let compKey = "completion:\(compId)"
            let compHeads = headsMap[compKey] ?? []
            let prevWinner = compHeads.last
            let prevDone = (prevWinner != nil && !prevWinner!.deleted)
                ? ((prevWinner!.payload["completed"] as? Bool) ?? false)
                : false
            let parentIds = prevWinner.map { [$0.revisionId] } ?? []
            newRev = [
                "revisionId": revisionId,
                "entityType": "completion",
                "entityId": compId,
                "parentRevisionIds": parentIds,
                "deviceId": deviceId,
                "modifiedAt": time,
                "logicalCounter": counter,
                "deleted": false,
                "payload": [
                    "taskId": taskId,
                    "date": today,
                    "completed": !prevDone,
                ],
            ]
            entityKey = compKey
        } else {
            let prevDone = (taskWinner.payload["completed"] as? Bool) ?? false
            var payload = taskWinner.payload
            payload["completed"] = !prevDone
            newRev = [
                "revisionId": revisionId,
                "entityType": "task",
                "entityId": taskId,
                "parentRevisionIds": [taskWinner.revisionId],
                "deviceId": deviceId,
                "modifiedAt": time,
                "logicalCounter": counter,
                "deleted": false,
                "payload": payload,
            ]
            entityKey = "task:\(taskId)"
        }

        rawRevs.append(newRev)
        outbox.append(revisionId)
        doc["revisions"] = rawRevs
        doc["outbox"] = outbox
        let nextGen = ((doc["localGeneration"] as? NSNumber)?.int64Value ?? 0) + 1
        doc["localGeneration"] = nextGen

        guard let updatedData = try? JSONSerialization.data(withJSONObject: doc),
              let updatedJson = String(data: updatedData, encoding: .utf8),
              let revData = try? JSONSerialization.data(withJSONObject: newRev),
              let revJson = String(data: revData, encoding: .utf8)
        else {
            rollback()
            return projectSnapshot(doc: doc, today: today)
        }

        let okDoc = execBind(
            db: db,
            sql: "INSERT INTO document(id,json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json",
            params: [updatedJson]
        )
        let okRev = execBind(
            db: db,
            sql: "INSERT OR IGNORE INTO revisions(id,entity_key,json) VALUES(?1,?2,?3)",
            params: [revisionId, entityKey, revJson]
        )
        let okOut = execBind(
            db: db,
            sql: "INSERT OR IGNORE INTO outbox(revision_id) VALUES(?1)",
            params: [revisionId]
        )

        if okDoc && okRev && okOut && sqlite3_exec(db, "COMMIT;", nil, nil, nil) == SQLITE_OK {
            return projectSnapshot(doc: doc, today: today)
        } else {
            rollback()
            return loadSnapshot(today: today)
        }
    }

    private static func queryDocumentJson(db: OpaquePointer) -> String? {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "SELECT json FROM document WHERE id=1", -1, &stmt, nil) == SQLITE_OK,
              let stmt = stmt
        else {
            return nil
        }
        defer { sqlite3_finalize(stmt) }
        guard sqlite3_step(stmt) == SQLITE_ROW, let cStr = sqlite3_column_text(stmt, 0) else {
            return nil
        }
        return String(cString: cStr)
    }

    private static func execBind(db: OpaquePointer, sql: String, params: [String]) -> Bool {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK, let stmt = stmt else {
            return false
        }
        defer { sqlite3_finalize(stmt) }
        for (idx, val) in params.enumerated() {
            sqlite3_bind_text(stmt, Int32(idx + 1), val, -1, sqliteTransient)
        }
        return sqlite3_step(stmt) == SQLITE_DONE
    }

    private static func compareRevision(_ a: RawRevision, _ b: RawRevision) -> Bool {
        if a.modifiedAt != b.modifiedAt { return a.modifiedAt < b.modifiedAt }
        if a.logicalCounter != b.logicalCounter { return a.logicalCounter < b.logicalCounter }
        if a.deviceId != b.deviceId { return a.deviceId < b.deviceId }
        return a.revisionId < b.revisionId
    }

    private static func entityHeads(_ revisions: [RawRevision]) -> [String: [RawRevision]] {
        var byId: [String: RawRevision] = [:]
        for r in revisions { byId[r.revisionId] = r }

        var deps: [String: Int] = [:]
        var children: [String: [String]] = [:]
        var ready: [String] = []
        for r in revisions {
            var missing = 0
            for pid in r.parentRevisionIds {
                missing += 1
                if let p = byId[pid], p.entityKey == r.entityKey {
                    children[pid, default: []].append(r.revisionId)
                }
            }
            deps[r.revisionId] = missing
            if missing == 0 { ready.append(r.revisionId) }
        }

        var valid = Set<String>()
        var idx = 0
        while idx < ready.count {
            let id = ready[idx]
            idx += 1
            valid.insert(id)
            for child in children[id] ?? [] {
                let next = (deps[child] ?? 1) - 1
                deps[child] = next
                if next == 0 { ready.append(child) }
            }
        }

        var parents = Set<String>()
        for r in revisions where valid.contains(r.revisionId) {
            for pid in r.parentRevisionIds { parents.insert(pid) }
        }

        var groups: [String: [RawRevision]] = [:]
        for r in revisions where valid.contains(r.revisionId) && !parents.contains(r.revisionId) {
            groups[r.entityKey, default: []].append(r)
        }
        for key in groups.keys {
            groups[key]?.sort(by: compareRevision)
        }
        return groups
    }

    private static func parseIsoDate(_ s: String) -> Date? {
        let df = DateFormatter()
        df.locale = Locale(identifier: "en_US_POSIX")
        df.timeZone = TimeZone(secondsFromGMT: 0)
        df.dateFormat = "yyyy-MM-dd"
        return df.date(from: s)
    }

    private static func shiftIsoDate(_ s: String, days: Int) -> String {
        guard let d = parseIsoDate(s),
              let shifted = Calendar(identifier: .gregorian).date(byAdding: .day, value: days, to: d)
        else { return s }
        let df = DateFormatter()
        df.locale = Locale(identifier: "en_US_POSIX")
        df.timeZone = TimeZone(secondsFromGMT: 0)
        df.dateFormat = "yyyy-MM-dd"
        return df.string(from: shifted)
    }

    private static func daysBetween(_ from: String, _ to: String) -> Int? {
        guard let a = parseIsoDate(from), let b = parseIsoDate(to) else { return nil }
        return Int(round(b.timeIntervalSince(a) / 86400.0))
    }

    private static func projectSnapshot(doc: [String: Any], today: String) -> WidgetSnapshotData {
        guard let rawRevs = doc["revisions"] as? [[String: Any]] else {
            return .empty(today: today)
        }
        let revisions = rawRevs.compactMap(RawRevision.init)
        let heads = entityHeads(revisions)
        var current: [String: RawRevision] = [:]
        for (k, list) in heads {
            if let winner = list.last { current[k] = winner }
        }

        var projects: [String: (String, String)] = [:]
        var completions: [String: Bool] = [:]
        var orders: [String: Double] = [:]

        for r in current.values where !r.deleted {
            if r.entityType == "projects" {
                let name = (r.payload["name"] as? String) ?? ""
                let color = (r.payload["color"] as? String) ?? ""
                projects[r.entityId] = (name, color)
            } else if r.entityType == "completion" {
                if let done = r.payload["completed"] as? Bool {
                    completions[r.entityId] = done
                }
            } else if r.entityType == "order" {
                if let pos = (r.payload["position"] as? NSNumber)?.doubleValue {
                    orders[r.entityId] = pos
                }
            }
        }

        var byDay: [[(WidgetTaskItem, Double)]] = [[], [], []]
        var daily: [(WidgetTaskItem, Double)] = []

        for r in current.values where r.entityType == "task" && !r.deleted {
            let p = r.payload
            guard let id = p["id"] as? String else { continue }
            let title = (p["title"] as? String) ?? ""
            let kind = (p["kind"] as? String) ?? "one_off"
            let time = p["time"] as? String
            let createdAt = (p["createdAt"] as? NSNumber)?.doubleValue ?? 0
            let projId = p["projectId"] as? String
            let proj = projId.flatMap { projects[$0] }

            if kind == "daily" {
                let done = completions["\(id)/\(today)"] ?? false
                var day = done ? today : shiftIsoDate(today, days: -1)
                var streak = 0
                while completions["\(id)/\(day)"] == true {
                    streak += 1
                    day = shiftIsoDate(day, days: -1)
                }
                if let legacy = p["legacyStreak"] as? [String: Any],
                   completions["\(id)/\(day)"] == nil,
                   let lDate = legacy["date"] as? String,
                   let lCount = (legacy["count"] as? NSNumber)?.intValue
                {
                    let start = shiftIsoDate(lDate, days: 1 - lCount)
                    if day >= start && day <= lDate, let diff = daysBetween(start, day) {
                        streak += max(0, diff + 1)
                    }
                }
                daily.append((
                    WidgetTaskItem(
                        id: id,
                        title: title,
                        time: time,
                        completed: done,
                        kind: "daily",
                        projectName: proj?.0,
                        projectColor: proj?.1,
                        streak: streak
                    ),
                    createdAt
                ))
            } else {
                let done = (p["completed"] as? Bool) ?? false
                guard let scheduledDate = p["scheduledDate"] as? String,
                      let offset = daysBetween(today, scheduledDate)
                else { continue }
                if offset < 0 && done { continue }
                if offset <= 2 {
                    let bucket = max(0, offset)
                    byDay[bucket].append((
                        WidgetTaskItem(
                            id: id,
                            title: title,
                            time: time,
                            completed: done,
                            kind: "one_off",
                            projectName: proj?.0,
                            projectColor: proj?.1,
                            streak: 0
                        ),
                        createdAt
                    ))
                }
            }
        }

        func sortBucket(_ list: inout [(WidgetTaskItem, Double)]) {
            list.sort { a, b in
                if a.0.completed != b.0.completed { return !a.0.completed }
                let pa = orders[a.0.id] ?? -a.1
                let pb = orders[b.0.id] ?? -b.1
                if pa != pb { return pa < pb }
                if a.1 != b.1 { return a.1 > b.1 }
                return a.0.id < b.0.id
            }
        }

        for i in 0..<3 { sortBucket(&byDay[i]) }
        sortBucket(&daily)

        var recent: [WidgetTaskItem] = []
        outer: for bucket in byDay {
            for (item, _) in bucket where !item.completed {
                recent.append(item)
                if recent.count >= 3 { break outer }
            }
        }

        let todayTasks = byDay[0].map(\.0)
        let dailyTasks = daily.map(\.0)
        let incomplete = todayTasks.filter { !$0.completed }.count
        let completed = todayTasks.filter { $0.completed }.count

        return WidgetSnapshotData(
            today: today,
            incompleteCount: incomplete,
            completedCount: completed,
            todayTasks: todayTasks,
            recentTasks: recent,
            dailyTasks: dailyTasks
        )
    }
}
