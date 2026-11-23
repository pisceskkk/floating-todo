import Foundation
import WidgetKit

private func resolveAppGroupContainerURL() -> URL? {
    var candidates: [String] = []
    if let bundleId = Bundle.main.bundleIdentifier, !bundleId.isEmpty {
        // Supports both standard Xcode signing (com.gaopengfei.floating-todo)
        // and SideStore/AltStore re-signing (com.gaopengfei.floating-todo.<TEAMID>).
        candidates.append("group.\(bundleId)")
    }
    if !candidates.contains("group.com.gaopengfei.floating-todo") {
        candidates.append("group.com.gaopengfei.floating-todo")
    }
    for groupId in candidates {
        if let url = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: groupId) {
            return url
        }
    }
    return nil
}

@_cdecl("floating_todo_ios_app_group_dir")
public func floatingTodoIosAppGroupDir() -> UnsafeMutablePointer<CChar>? {
    guard let url = resolveAppGroupContainerURL() else { return nil }
    return strdup(url.path)
}

@_cdecl("floating_todo_ios_free_string")
public func floatingTodoIosFreeString(_ ptr: UnsafeMutablePointer<CChar>?) {
    if let ptr = ptr {
        free(ptr)
    }
}

@_cdecl("floating_todo_ios_reload_widgets")
public func floatingTodoIosReloadWidgets() {
    WidgetCenter.shared.reloadAllTimelines()
}
