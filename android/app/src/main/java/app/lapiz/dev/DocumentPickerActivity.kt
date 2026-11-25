package app.lapiz.dev

import android.app.AlertDialog
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.webkit.MimeTypeMap
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts

class DocumentPickerActivity : ComponentActivity() {
    private data class FormatFilter(val label: String, val extensions: List<String>)

    private lateinit var launcher: ActivityResultLauncher<Intent>
    private var formatDialog: AlertDialog? = null
    private var pickerLaunched = false
    private var selectedFilterIndex = 0
    private var delivered = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        launcher = registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
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

        if (savedInstanceState != null) {
            pickerLaunched = savedInstanceState.getBoolean(STATE_PICKER_LAUNCHED, false)
            selectedFilterIndex = savedInstanceState.getInt(STATE_SELECTED_FILTER, 0)
        }

        if (pickerLaunched) {
            return
        }

        val name = intent.getStringExtra(EXTRA_NAME) ?: ""
        val save = intent.getBooleanExtra(EXTRA_SAVE, false)
        if (!save) {
            launchPicker(false, name, null)
            return
        }

        val filters = parseFilters(
            intent.getStringArrayExtra(EXTRA_FILTER_LABELS),
            intent.getStringArrayExtra(EXTRA_FILTER_EXTENSIONS),
        )
        when {
            filters.size > 1 -> {
                if (savedInstanceState == null) {
                    selectedFilterIndex = defaultFilterIndex(name, filters)
                } else {
                    selectedFilterIndex = selectedFilterIndex.coerceIn(0, filters.lastIndex)
                }
                showFormatDialog(name, filters)
            }
            filters.size == 1 -> {
                val extension = filters[0].extensions.first()
                launchPicker(true, applyExtension(name, extension, filters), extension)
            }
            else -> {
                launchPicker(true, name, extractExtension(name))
            }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putBoolean(STATE_PICKER_LAUNCHED, pickerLaunched)
        outState.putInt(STATE_SELECTED_FILTER, selectedFilterIndex)
    }

    override fun onDestroy() {
        formatDialog?.dismiss()
        formatDialog = null
        super.onDestroy()
    }

    private fun showFormatDialog(name: String, filters: List<FormatFilter>) {
        val labels = filters.map { it.label }.toTypedArray()
        formatDialog = AlertDialog.Builder(this)
            .setSingleChoiceItems(labels, selectedFilterIndex) { _, which ->
                selectedFilterIndex = which
            }
            .setPositiveButton(android.R.string.ok) { _, _ ->
                val chosen = filters[selectedFilterIndex.coerceIn(0, filters.lastIndex)]
                val extension = chosen.extensions.first()
                launchPicker(true, applyExtension(name, extension, filters), extension)
            }
            .setNegativeButton(android.R.string.cancel) { _, _ ->
                deliver(0, null, null)
            }
            .setOnCancelListener {
                deliver(0, null, null)
            }
            .show()
    }

    private fun launchPicker(save: Boolean, name: String, extension: String?) {
        pickerLaunched = true
        val picker = Intent(if (save) Intent.ACTION_CREATE_DOCUMENT else Intent.ACTION_OPEN_DOCUMENT).apply {
            addCategory(Intent.CATEGORY_OPENABLE)
            putExtra(Intent.EXTRA_LOCAL_ONLY, true)
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION,
            )
            if (save) {
                type = mimeTypeForExtension(extension)
                putExtra(Intent.EXTRA_TITLE, name)
            } else {
                type = "*/*"
            }
        }

        try {
            launcher.launch(picker)
        } catch (_: Exception) {
            deliver(-1, null, null)
        }
    }

    // -1 = error 0 = cancelled 1 = success
    private fun deliver(status: Int, uri: Uri?, name: String?, grantFlags: Int = 0) {
        if (delivered) {
            return
        }
        delivered = true
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
        private const val EXTRA_FILTER_LABELS = "app.lapiz.dev.FILTER_LABELS"
        private const val EXTRA_FILTER_EXTENSIONS = "app.lapiz.dev.FILTER_EXTENSIONS"
        private const val EXTRA_STATUS = "app.lapiz.dev.STATUS"
        private const val STATE_PICKER_LAUNCHED = "app.lapiz.dev.STATE_PICKER_LAUNCHED"
        private const val STATE_SELECTED_FILTER = "app.lapiz.dev.STATE_SELECTED_FILTER"

        internal data class Result(val request: Long, val status: Int, val uri: String?, val name: String?)

        internal fun createIntent(
            context: Context,
            request: Long,
            save: Boolean,
            name: String,
            filterLabels: Array<String>,
            filterExtensions: Array<String>,
        ): Intent =
            Intent(context, DocumentPickerActivity::class.java).apply {
                putExtra(EXTRA_REQUEST, request)
                putExtra(EXTRA_SAVE, save)
                putExtra(EXTRA_NAME, name)
                putExtra(EXTRA_FILTER_LABELS, filterLabels)
                putExtra(EXTRA_FILTER_EXTENSIONS, filterExtensions)
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

        private fun parseFilters(
            labels: Array<String>?,
            extensions: Array<String>?,
        ): List<FormatFilter> {
            if (labels == null || extensions == null) {
                return emptyList()
            }
            val count = minOf(labels.size, extensions.size)
            val result = ArrayList<FormatFilter>(count)
            for (i in 0 until count) {
                val parsedExtensions = extensions[i]
                    .split(',')
                    .map { it.trim().trimStart('.') }
                    .filter { it.isNotEmpty() }
                if (parsedExtensions.isNotEmpty()) {
                    result.add(FormatFilter(labels[i], parsedExtensions))
                }
            }
            return result
        }

        private fun extractExtension(name: String): String? {
            val dot = name.lastIndexOf('.')
            if (dot <= 0 || dot == name.lastIndex) {
                return null
            }
            return name.substring(dot + 1)
        }

        private fun defaultFilterIndex(name: String, filters: List<FormatFilter>): Int {
            val extension = extractExtension(name) ?: return 0
            val index = filters.indexOfFirst { filter ->
                filter.extensions.any { it.equals(extension, ignoreCase = true) }
            }
            return if (index >= 0) index else 0
        }

        private fun applyExtension(
            name: String,
            extension: String,
            filters: List<FormatFilter>,
        ): String {
            val existing = extractExtension(name)
            val stem = if (existing != null && filters.any { filter ->
                    filter.extensions.any { it.equals(existing, ignoreCase = true) }
                }
            ) {
                name.substring(0, name.length - existing.length - 1)
            } else {
                name
            }
            val base = stem.ifEmpty { "Untitled" }
            return "$base.$extension"
        }

        private fun mimeTypeForExtension(extension: String?): String {
            if (extension.isNullOrBlank()) {
                return "application/octet-stream"
            }
            return MimeTypeMap.getSingleton()
                .getMimeTypeFromExtension(extension.lowercase())
                ?: "application/octet-stream"
        }
    }
}
