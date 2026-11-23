import SwiftUI
import WidgetKit
#if canImport(AppIntents)
import AppIntents
#endif

@available(iOS 17.0, *)
struct ToggleTodoIntent: AppIntent {
    static var title: LocalizedStringResource = "切换待办状态"

    @Parameter(title: "任务 ID")
    var taskId: String

    init() {
        self.taskId = ""
    }

    init(taskId: String) {
        self.taskId = taskId
    }

    func perform() async throws -> some IntentResult {
        if !taskId.isEmpty {
            TodoWidgetStore.toggleTask(taskId: taskId)
        }
        return .result()
    }
}

struct TodoWidgetEntry: TimelineEntry {
    let date: Date
    let snapshot: WidgetSnapshotData
}

struct TodoWidgetProvider: TimelineProvider {
    func placeholder(in context: Context) -> TodoWidgetEntry {
        TodoWidgetEntry(
            date: Date(),
            snapshot: WidgetSnapshotData(
                today: TodoWidgetStore.todayIso(),
                incompleteCount: 2,
                completedCount: 1,
                todayTasks: [
                    WidgetTaskItem(id: "p1", title: "提交周报", time: "15:30", completed: false, kind: "one_off", projectName: "工作", projectColor: "#007aff", streak: 0),
                    WidgetTaskItem(id: "p2", title: "晚上健身", time: "19:00", completed: false, kind: "one_off", projectName: nil, projectColor: nil, streak: 0),
                ],
                recentTasks: [],
                dailyTasks: [
                    WidgetTaskItem(id: "d1", title: "阅读 30 分钟", time: nil, completed: true, kind: "daily", projectName: nil, projectColor: nil, streak: 7),
                ]
            )
        )
    }

    func getSnapshot(in context: Context, completion: @escaping (TodoWidgetEntry) -> Void) {
        completion(TodoWidgetEntry(date: Date(), snapshot: TodoWidgetStore.loadSnapshot()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<TodoWidgetEntry>) -> Void) {
        let entry = TodoWidgetEntry(date: Date(), snapshot: TodoWidgetStore.loadSnapshot())
        let nextRefresh = Calendar.current.date(byAdding: .minute, value: 30, to: Date()) ?? Date().addingTimeInterval(1800)
        completion(Timeline(entries: [entry], policy: .after(nextRefresh)))
    }
}

struct TodoWidgetEntryView: View {
    @Environment(\.widgetFamily) private var family
    let entry: TodoWidgetEntry

    private var quickCaptureURL: URL {
        URL(string: "floatingtodo://quick-capture")!
    }

    private var openTodayURL: URL {
        URL(string: "floatingtodo://open?day=today")!
    }

    var body: some View {
        let snap = entry.snapshot
        let tasks = snap.todayTasks.isEmpty ? snap.recentTasks : snap.todayTasks
        let maxTasks = family == .systemSmall ? 2 : (family == .systemLarge ? 5 : 3)

        VStack(alignment: .leading, spacing: family == .systemSmall ? 6 : 7) {
            HStack(spacing: 6) {
                VStack(alignment: .leading, spacing: 1) {
                    Text("今日待办")
                        .font(.system(size: family == .systemSmall ? 14 : 15, weight: .bold))
                        .foregroundColor(.primary)
                    Text(snap.completedCount > 0 ? "\(snap.incompleteCount) 待办 · \(snap.completedCount) 已完成" : "\(snap.incompleteCount) 件待办")
                        .font(.system(size: 11))
                        .foregroundColor(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 4)
                Link(destination: quickCaptureURL) {
                    Image(systemName: "plus")
                        .font(.system(size: 13, weight: .bold))
                        .foregroundColor(.white)
                        .frame(width: 26, height: 26)
                        .background(Color.accentColor)
                        .clipShape(Circle())
                }
            }

            if tasks.isEmpty {
                Spacer(minLength: 4)
                Text("今天没有待办")
                    .font(.system(size: 12))
                    .foregroundColor(.secondary)
                Spacer(minLength: 4)
            } else {
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(Array(tasks.prefix(maxTasks))) { task in
                        taskRow(task: task, showMeta: family != .systemSmall)
                    }
                }
            }

            if family == .systemLarge && !snap.dailyTasks.isEmpty {
                Divider().padding(.vertical, 2)
                Text("每日打卡")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundColor(.secondary)
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(Array(snap.dailyTasks.prefix(3))) { habit in
                        dailyRow(habit: habit)
                    }
                }
            }

            Spacer(minLength: 0)
        }
        .padding(family == .systemSmall ? 12 : 14)
        .widgetURL(openTodayURL)
    }

    @ViewBuilder
    private func taskRow(task: WidgetTaskItem, showMeta: Bool) -> some View {
        HStack(spacing: 7) {
            checkButton(taskId: task.id, completed: task.completed)
            Text(task.title)
                .font(.system(size: 13))
                .foregroundColor(task.completed ? .secondary : .primary)
                .strikethrough(task.completed)
                .lineLimit(1)
            Spacer(minLength: 2)
            if showMeta, let time = task.time, !time.isEmpty {
                Text(time)
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(.secondary)
            }
        }
    }

    @ViewBuilder
    private func dailyRow(habit: WidgetTaskItem) -> some View {
        HStack(spacing: 7) {
            checkButton(taskId: habit.id, completed: habit.completed)
            Text(habit.title)
                .font(.system(size: 12.5))
                .foregroundColor(habit.completed ? .secondary : .primary)
                .lineLimit(1)
            Spacer(minLength: 2)
            if habit.streak > 0 {
                Text("🔥 \(habit.streak)天")
                    .font(.system(size: 11))
                    .foregroundColor(.secondary)
            }
        }
    }

    @ViewBuilder
    private func checkButton(taskId: String, completed: Bool) -> some View {
        if #available(iOS 17.0, *) {
            Button(intent: ToggleTodoIntent(taskId: taskId)) {
                Image(systemName: completed ? "checkmark.circle.fill" : "circle")
                    .font(.system(size: 16))
                    .foregroundColor(completed ? .secondary : .accentColor)
            }
            .buttonStyle(.plain)
        } else {
            Image(systemName: completed ? "checkmark.circle.fill" : "circle")
                .font(.system(size: 16))
                .foregroundColor(completed ? .secondary : .accentColor)
        }
    }
}

@main
struct TodoWidget: Widget {
    let kind: String = "TodoWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: TodoWidgetProvider()) { entry in
            if #available(iOS 17.0, *) {
                TodoWidgetEntryView(entry: entry)
                    .containerBackground(.background, for: .widget)
            } else {
                TodoWidgetEntryView(entry: entry)
                    .background(Color(UIColor.systemBackground))
            }
        }
        .configurationDisplayName("悬浮待办")
        .description("查看今日待办与每日打卡，支持桌面直接勾选完成与快速新建。")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
    }
}
