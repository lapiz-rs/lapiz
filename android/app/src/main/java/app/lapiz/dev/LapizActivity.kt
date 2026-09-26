package app.lapiz.dev

import android.app.NativeActivity
import android.content.Intent
import android.os.Bundle
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat

class LapizActivity : NativeActivity() {
    @Volatile
    private var documentRequest = 0L

    @Synchronized
    fun requestDocument(request: Long, save: Boolean, name: String): Boolean {
        if (documentRequest != 0L || isFinishing || isDestroyed) {
            return false
        }
        documentRequest = request
        runOnUiThread {
            if (documentRequest != request) {
                return@runOnUiThread
            }
            try {
                startActivity(DocumentPickerActivity.createIntent(this, request, save, name))
            } catch (_: Exception) {
                completeDocument(-1, null, null)
            }
        }
        return true
    }

    @Synchronized
    private fun completeDocument(status: Int, uri: String?, name: String?) {
        val request = documentRequest
        documentRequest = 0L
        if (request != 0L) {
            onDocumentPickedNative(request, status, uri, name)
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        val result = DocumentPickerActivity.readResult(intent) ?: return
        if (result.request == documentRequest) {
            completeDocument(result.status, result.uri, result.name)
        }
    }

    fun copyDocument(uri: String, path: String): Boolean =
        DocumentFiles.copy(contentResolver, uri, path)

    fun writeDocument(uri: String, path: String): Boolean =
        DocumentFiles.write(contentResolver, uri, path)

    override fun onDestroy() {
        completeDocument(0, null, null)
        super.onDestroy()
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        hideSystemBars()
    }

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (hasFocus) {
            hideSystemBars()
        }
    }

    private fun hideSystemBars() {
        WindowCompat.setDecorFitsSystemWindows(window, false)
        WindowCompat.getInsetsController(window, window.decorView).apply {
            systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
            hide(WindowInsetsCompat.Type.systemBars())
        }
    }

    companion object {
        @JvmStatic
        private external fun onDocumentPickedNative(request: Long, status: Int, uri: String?, name: String?)
    }
}
