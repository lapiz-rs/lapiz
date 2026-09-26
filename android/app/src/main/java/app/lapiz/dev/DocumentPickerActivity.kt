package app.lapiz.dev

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.webkit.MimeTypeMap
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts

class DocumentPickerActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        val launcher = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
            val data = result.data
            val uri = data?.data
            if (result.resultCode != RESULT_OK || data == null || uri == null) {
                deliver(0, null, null)
                return@registerForActivityResult
            }

            val flags = data.flags and
                (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
            if (flags != 0) {
                try {
                    contentResolver.takePersistableUriPermission(uri, flags)
                } catch (_: SecurityException) {
                }
            }
            val name = DocumentFiles.displayName(contentResolver, uri)
            deliver(if (name == null) -1 else 1, uri, name, flags)
        }

        if (savedInstanceState == null) {
            val name = intent.getStringExtra(EXTRA_NAME) ?: ""
            val save = intent.getBooleanExtra(EXTRA_SAVE, false)
            val picker = Intent(if (save) Intent.ACTION_CREATE_DOCUMENT else Intent.ACTION_OPEN_DOCUMENT).apply {
                addCategory(Intent.CATEGORY_OPENABLE)
                addFlags(
                    Intent.FLAG_GRANT_READ_URI_PERMISSION or
                        Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                        Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
                )
                val extension = MimeTypeMap.getFileExtensionFromUrl(name)
                type = MimeTypeMap.getSingleton().getMimeTypeFromExtension(extension) ?: "*/*"
                if (save) {
                    putExtra(Intent.EXTRA_TITLE, name)
                }
            }

            try {
                launcher.launch(picker)
            } catch (_: Exception) {
                deliver(-1, null, null)
            }
        }
    }

    // -1 = error 0 = cancelled 1 = success
    private fun deliver(status: Int, uri: Uri?, name: String?, grantFlags: Int = 0) {
        startActivity(Intent(this, LapizActivity::class.java).apply {
            addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP or grantFlags)
            putExtra(EXTRA_REQUEST, intent.getLongExtra(EXTRA_REQUEST, 0L))
            putExtra(EXTRA_STATUS, status)
            data = uri
            putExtra(EXTRA_NAME, name)
        })
        finish()
    }

    companion object {
        private const val EXTRA_REQUEST = "app.lapiz.dev.REQUEST"
        private const val EXTRA_SAVE = "app.lapiz.dev.SAVE"
        private const val EXTRA_NAME = "app.lapiz.dev.NAME"
        private const val EXTRA_STATUS = "app.lapiz.dev.STATUS"

        internal data class Result(val request: Long, val status: Int, val uri: String?, val name: String?)

        internal fun createIntent(context: Context, request: Long, save: Boolean, name: String): Intent =
            Intent(context, DocumentPickerActivity::class.java).apply {
                putExtra(EXTRA_REQUEST, request)
                putExtra(EXTRA_SAVE, save)
                putExtra(EXTRA_NAME, name)
            }

        internal fun readResult(intent: Intent): Result? {
            if (!intent.hasExtra(EXTRA_REQUEST) || !intent.hasExtra(EXTRA_STATUS)) {
                return null
            }
            return Result(
                intent.getLongExtra(EXTRA_REQUEST, 0L),
                intent.getIntExtra(EXTRA_STATUS, -1),
                intent.data?.toString(),
                intent.getStringExtra(EXTRA_NAME),
            )
        }
    }
}
