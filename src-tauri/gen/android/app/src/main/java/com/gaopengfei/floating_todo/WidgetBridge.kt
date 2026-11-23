package com.gaopengfei.floating_todo

import android.content.Context
import android.os.Build
import android.util.Log
import org.json.JSONObject
import java.io.File
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

object WidgetBridge {
  private const val TAG = "FloatingTodoWidget"

  @Volatile
  private var loaded = false

  private fun ensureLoaded(): Boolean {
    if (loaded) return true
    return try {
      System.loadLibrary("floating_todo_lib")
      loaded = true
      true
    } catch (t: Throwable) {
      Log.e(TAG, "Failed to load floating_todo_lib", t)
      false
    }
  }

  @JvmStatic
  private external fun nativeLoadSnapshot(dbPath: String, today: String): String

  @JvmStatic
  private external fun nativeToggleTask(dbPath: String, taskId: String, today: String): String

  fun dbPath(context: Context): String {
    // Tauri v2's app.path().app_data_dir() on Android resolves to context.dataDir
    // (NOT context.filesDir). Prefer dataDir and fall back to filesDir if present.
    val dataDir = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
      context.dataDir
    } else {
      File(context.applicationInfo.dataDir)
    }
    val primary = File(dataDir, "todo-v3.sqlite3")
    if (primary.exists()) return primary.absolutePath
    val fallback = File(context.filesDir, "todo-v3.sqlite3")
    if (fallback.exists()) return fallback.absolutePath
    return primary.absolutePath
  }

  fun todayIso(): String {
    return SimpleDateFormat("yyyy-MM-dd", Locale.US).format(Date())
  }

  fun loadSnapshot(context: Context): JSONObject? {
    if (!ensureLoaded()) return null
    return try {
      val path = dbPath(context)
      val raw = nativeLoadSnapshot(path, todayIso())
      val parsed = JSONObject(raw)
      if (parsed.optBoolean("ok", false)) {
        parsed.optJSONObject("snapshot")
      } else {
        Log.w(TAG, "nativeLoadSnapshot error ($path): ${parsed.optString("error")}")
        null
      }
    } catch (t: Throwable) {
      Log.e(TAG, "loadSnapshot failed", t)
      null
    }
  }

  fun toggleTask(context: Context, taskId: String): JSONObject? {
    if (!ensureLoaded()) return null
    return try {
      val path = dbPath(context)
      val raw = nativeToggleTask(path, taskId, todayIso())
      val parsed = JSONObject(raw)
      if (parsed.optBoolean("ok", false)) {
        parsed.optJSONObject("snapshot")
      } else {
        Log.w(TAG, "nativeToggleTask error ($path): ${parsed.optString("error")}")
        null
      }
    } catch (t: Throwable) {
      Log.e(TAG, "toggleTask failed", t)
      null
    }
  }
}
