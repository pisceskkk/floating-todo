package com.gaopengfei.floating_todo

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.Paint
import android.net.Uri
import android.os.Bundle
import android.view.View
import android.widget.RemoteViews
import android.widget.Toast
import androidx.core.content.ContextCompat
import org.json.JSONArray
import org.json.JSONObject

class TodoWidgetProvider : AppWidgetProvider() {

  override fun onUpdate(
    context: Context,
    appWidgetManager: AppWidgetManager,
    appWidgetIds: IntArray
  ) {
    val snapshot = WidgetBridge.loadSnapshot(context)
    for (id in appWidgetIds) {
      val options = appWidgetManager.getAppWidgetOptions(id)
      renderWidget(context, appWidgetManager, id, options, snapshot)
    }
  }

  override fun onAppWidgetOptionsChanged(
    context: Context,
    appWidgetManager: AppWidgetManager,
    appWidgetId: Int,
    newOptions: Bundle
  ) {
    val snapshot = WidgetBridge.loadSnapshot(context)
    renderWidget(context, appWidgetManager, appWidgetId, newOptions, snapshot)
  }

  override fun onReceive(context: Context, intent: Intent) {
    super.onReceive(context, intent)
    when (intent.action) {
      ACTION_TOGGLE_TASK -> {
        val taskId = intent.getStringExtra(EXTRA_TASK_ID) ?: return
        val snapshot = WidgetBridge.toggleTask(context, taskId) ?: WidgetBridge.loadSnapshot(context)
        updateAll(context, snapshot)
      }
      ACTION_REFRESH -> {
        val snapshot = WidgetBridge.loadSnapshot(context)
        updateAll(context, snapshot)
        val count = snapshot?.optInt("incompleteCount", 0)
        val msg = if (count != null) "已刷新（$count 件待办）" else "刷新失败，请先打开主应用"
        Toast.makeText(context, msg, Toast.LENGTH_SHORT).show()
      }
    }
  }

  private fun updateAll(context: Context, snapshot: JSONObject?) {
    val manager = AppWidgetManager.getInstance(context)
    val component = ComponentName(context, TodoWidgetProvider::class.java)
    val ids = manager.getAppWidgetIds(component)
    for (id in ids) {
      val options = manager.getAppWidgetOptions(id)
      renderWidget(context, manager, id, options, snapshot)
    }
  }

  private fun renderWidget(
    context: Context,
    manager: AppWidgetManager,
    appWidgetId: Int,
    options: Bundle,
    snapshot: JSONObject?
  ) {
    val views = RemoteViews(context.packageName, R.layout.widget_todo)
    val minHeightDp = options.getInt(AppWidgetManager.OPTION_APPWIDGET_MIN_HEIGHT, 160)
    val isSmall = minHeightDp < 140
    val isLarge = minHeightDp >= 220
    val maxTaskRows = when {
      isSmall -> 2
      isLarge -> 5
      else -> 3
    }

    // Open app & quick-capture intents
    val openIntent = Intent(Intent.ACTION_VIEW, Uri.parse("floatingtodo://open?day=today")).apply {
      setClass(context, MainActivity::class.java)
      flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
    }
    val openPi = PendingIntent.getActivity(
      context,
      appWidgetId * 100 + 1,
      openIntent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    )
    views.setOnClickPendingIntent(R.id.widget_title_area, openPi)
    views.setOnClickPendingIntent(R.id.widget_empty, openPi)

    val addIntent = Intent(Intent.ACTION_VIEW, Uri.parse("floatingtodo://quick-capture")).apply {
      setClass(context, MainActivity::class.java)
      flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP
    }
    val addPi = PendingIntent.getActivity(
      context,
      appWidgetId * 100 + 2,
      addIntent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    )
    views.setOnClickPendingIntent(R.id.widget_add_btn, addPi)

    val refreshIntent = Intent(context, TodoWidgetProvider::class.java).apply {
      action = ACTION_REFRESH
      setPackage(context.packageName)
    }
    val refreshPi = PendingIntent.getBroadcast(
      context,
      appWidgetId * 100 + 3,
      refreshIntent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    )
    views.setOnClickPendingIntent(R.id.widget_refresh_btn, refreshPi)

    val incomplete = snapshot?.optInt("incompleteCount", 0) ?: 0
    val completed = snapshot?.optInt("completedCount", 0) ?: 0
    views.setTextViewText(R.id.widget_count, if (completed > 0) "$incomplete 待办 · $completed 已完成" else "$incomplete 件待办")

    val todayTasks = snapshot?.optJSONArray("todayTasks") ?: JSONArray()
    val recentTasks = snapshot?.optJSONArray("recentTasks") ?: JSONArray()
    val displayTasks = if (todayTasks.length() > 0) todayTasks else recentTasks

    val primaryColor = ContextCompat.getColor(context, R.color.widget_text_primary)
    val secondaryColor = ContextCompat.getColor(context, R.color.widget_text_secondary)
    val accentColor = ContextCompat.getColor(context, R.color.widget_accent)

    val taskRowIds = intArrayOf(
      R.id.widget_row_0, R.id.widget_row_1, R.id.widget_row_2, R.id.widget_row_3, R.id.widget_row_4
    )
    val taskCheckIds = intArrayOf(
      R.id.widget_check_0, R.id.widget_check_1, R.id.widget_check_2, R.id.widget_check_3, R.id.widget_check_4
    )
    val taskTitleIds = intArrayOf(
      R.id.widget_title_0, R.id.widget_title_1, R.id.widget_title_2, R.id.widget_title_3, R.id.widget_title_4
    )
    val taskMetaIds = intArrayOf(
      R.id.widget_meta_0, R.id.widget_meta_1, R.id.widget_meta_2, R.id.widget_meta_3, R.id.widget_meta_4
    )

    val visibleTaskCount = minOf(displayTasks.length(), maxTaskRows)
    views.setViewVisibility(R.id.widget_empty, if (visibleTaskCount == 0) View.VISIBLE else View.GONE)

    for (i in taskRowIds.indices) {
      if (i < visibleTaskCount) {
        val task = displayTasks.getJSONObject(i)
        val taskId = task.optString("id")
        val title = task.optString("title")
        val done = task.optBoolean("completed", false)
        val time = task.optString("time", "")
        val projectName = task.optString("projectName", "")
        val meta = buildString {
          if (time.isNotEmpty() && time != "null") append(time)
          if (projectName.isNotEmpty() && projectName != "null") {
            if (isNotEmpty()) append(" · ")
            append(projectName)
          }
        }

        views.setViewVisibility(taskRowIds[i], View.VISIBLE)
        views.setTextViewText(taskCheckIds[i], if (done) "✓" else "○")
        views.setTextColor(taskCheckIds[i], if (done) secondaryColor else accentColor)
        views.setTextViewText(taskTitleIds[i], title)
        views.setTextColor(taskTitleIds[i], if (done) secondaryColor else primaryColor)
        views.setInt(
          taskTitleIds[i],
          "setPaintFlags",
          if (done) Paint.ANTI_ALIAS_FLAG or Paint.STRIKE_THRU_TEXT_FLAG else Paint.ANTI_ALIAS_FLAG
        )
        views.setTextViewText(taskMetaIds[i], meta)
        views.setViewVisibility(taskMetaIds[i], if (meta.isNotEmpty()) View.VISIBLE else View.GONE)

        val togglePi = createTogglePendingIntent(context, appWidgetId * 100 + 10 + i, taskId)
        views.setOnClickPendingIntent(taskCheckIds[i], togglePi)
        views.setOnClickPendingIntent(taskTitleIds[i], openPi)
      } else {
        views.setViewVisibility(taskRowIds[i], View.GONE)
      }
    }

    // Daily habits section (shown on Large widgets)
    val dailyTasks = snapshot?.optJSONArray("dailyTasks") ?: JSONArray()
    val dailyRowIds = intArrayOf(R.id.widget_daily_row_0, R.id.widget_daily_row_1, R.id.widget_daily_row_2)
    val dailyCheckIds = intArrayOf(R.id.widget_daily_check_0, R.id.widget_daily_check_1, R.id.widget_daily_check_2)
    val dailyTitleIds = intArrayOf(R.id.widget_daily_title_0, R.id.widget_daily_title_1, R.id.widget_daily_title_2)
    val dailyStreakIds = intArrayOf(R.id.widget_daily_streak_0, R.id.widget_daily_streak_1, R.id.widget_daily_streak_2)

    val visibleDaily = if (isLarge) minOf(dailyTasks.length(), dailyRowIds.size) else 0
    views.setViewVisibility(R.id.widget_daily_header, if (visibleDaily > 0) View.VISIBLE else View.GONE)

    for (i in dailyRowIds.indices) {
      if (i < visibleDaily) {
        val habit = dailyTasks.getJSONObject(i)
        val habitId = habit.optString("id")
        val title = habit.optString("title")
        val done = habit.optBoolean("completed", false)
        val streak = habit.optInt("streak", 0)

        views.setViewVisibility(dailyRowIds[i], View.VISIBLE)
        views.setTextViewText(dailyCheckIds[i], if (done) "✓" else "○")
        views.setTextColor(dailyCheckIds[i], if (done) secondaryColor else accentColor)
        views.setTextViewText(dailyTitleIds[i], title)
        views.setTextColor(dailyTitleIds[i], if (done) secondaryColor else primaryColor)
        views.setTextViewText(dailyStreakIds[i], if (streak > 0) "🔥 ${streak}天" else "")
        views.setViewVisibility(dailyStreakIds[i], if (streak > 0) View.VISIBLE else View.GONE)

        val togglePi = createTogglePendingIntent(context, appWidgetId * 100 + 30 + i, habitId)
        views.setOnClickPendingIntent(dailyCheckIds[i], togglePi)
        views.setOnClickPendingIntent(dailyTitleIds[i], openPi)
      } else {
        views.setViewVisibility(dailyRowIds[i], View.GONE)
      }
    }

    manager.updateAppWidget(appWidgetId, views)
  }

  private fun createTogglePendingIntent(
    context: Context,
    requestCode: Int,
    taskId: String
  ): PendingIntent {
    val intent = Intent(context, TodoWidgetProvider::class.java).apply {
      action = ACTION_TOGGLE_TASK
      setPackage(context.packageName)
      putExtra(EXTRA_TASK_ID, taskId)
    }
    return PendingIntent.getBroadcast(
      context,
      requestCode,
      intent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    )
  }

  companion object {
    const val ACTION_TOGGLE_TASK = "com.gaopengfei.floating_todo.ACTION_TOGGLE_TASK"
    const val ACTION_REFRESH = "com.gaopengfei.floating_todo.ACTION_WIDGET_REFRESH"
    const val EXTRA_TASK_ID = "extra_task_id"

    fun requestRefresh(context: Context) {
      try {
        val manager = AppWidgetManager.getInstance(context)
        val component = ComponentName(context, TodoWidgetProvider::class.java)
        val ids = manager.getAppWidgetIds(component)
        if (ids.isEmpty()) return
        val intent = Intent(context, TodoWidgetProvider::class.java).apply {
          action = AppWidgetManager.ACTION_APPWIDGET_UPDATE
          putExtra(AppWidgetManager.EXTRA_APPWIDGET_IDS, ids)
        }
        context.sendBroadcast(intent)
      } catch (_: Throwable) {}
    }
  }
}
