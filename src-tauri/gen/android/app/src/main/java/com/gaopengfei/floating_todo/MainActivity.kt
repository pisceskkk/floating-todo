package com.gaopengfei.floating_todo

import android.content.Context
import android.content.Intent
import android.net.wifi.WifiManager
import android.os.Bundle
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge
import org.json.JSONObject

class MainActivity : TauriActivity() {
  private var multicastLock: WifiManager.MulticastLock? = null
  private var pendingDeepLink: String? = null
  private var webViewRef: WebView? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    captureDeepLink(intent)
  }

  override fun onWebViewCreate(webView: WebView) {
    super.onWebViewCreate(webView)
    webViewRef = webView
    flushPendingDeepLink()
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    setIntent(intent)
    captureDeepLink(intent)
    flushPendingDeepLink()
  }

  override fun onResume() {
    super.onResume()
    acquireMulticastLock()
    flushPendingDeepLink()
  }

  override fun onPause() {
    releaseMulticastLock()
    TodoWidgetProvider.requestRefresh(this)
    super.onPause()
  }

  private fun captureDeepLink(intent: Intent?) {
    val uri = intent?.data ?: return
    if (uri.scheme == "floatingtodo") {
      pendingDeepLink = uri.toString()
    }
  }

  private fun flushPendingDeepLink() {
    val url = pendingDeepLink ?: return
    val wv = webViewRef ?: return
    pendingDeepLink = null
    val quoted = JSONObject.quote(url)
    wv.post {
      wv.evaluateJavascript(
        "window.dispatchEvent(new CustomEvent('floatingtodo-deep-link', { detail: $quoted }));",
        null
      )
    }
  }

  private fun acquireMulticastLock() {
    try {
      if (multicastLock == null) {
        val wifi = applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager
        multicastLock = wifi?.createMulticastLock("floating-todo-mdns")?.apply {
          setReferenceCounted(false)
        }
      }
      if (multicastLock?.isHeld == false) {
        multicastLock?.acquire()
      }
    } catch (_: Throwable) {}
  }

  private fun releaseMulticastLock() {
    try {
      if (multicastLock?.isHeld == true) {
        multicastLock?.release()
      }
    } catch (_: Throwable) {}
  }
}
