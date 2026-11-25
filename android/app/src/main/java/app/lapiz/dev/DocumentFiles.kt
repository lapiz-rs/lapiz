package app.lapiz.dev

import android.content.ContentResolver
import android.net.Uri
import android.provider.OpenableColumns
import java.io.FileInputStream
import java.io.FileOutputStream
import androidx.core.net.toUri

internal object DocumentFiles {
    fun displayName(resolver: ContentResolver, uri: Uri): String? {
        var name: String? = null
        try {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
                ?.use { cursor ->
                    if (cursor.moveToFirst()) {
                        val index = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                        if (index >= 0) {
                            name = cursor.getString(index)
                        }
                    }
                }
        } catch (_: Exception) {
        }
        return if (name.isNullOrEmpty()) uri.lastPathSegment else name
    }

    fun openFd(resolver: ContentResolver, uri: String, mode: String): Int {
        return try {
            resolver.openFileDescriptor(uri.toUri(), mode)?.detachFd() ?: -1
        } catch (_: Exception) {
            -1
        }
    }

    fun copy(resolver: ContentResolver, uri: String, path: String): Boolean {
        return try {
            resolver.openInputStream(uri.toUri()).use { input ->
                FileOutputStream(path).use { output ->
                    if (input == null) {
                        return false
                    }
                    input.copyTo(output, 64 * 1024)
                }
            }
            true
        } catch (_: Exception) {
            false
        }
    }

    fun write(resolver: ContentResolver, uri: String, path: String): Boolean {
        return try {
            FileInputStream(path).use { input ->
                resolver.openOutputStream(uri.toUri(), "wt").use { output ->
                    if (output == null) {
                        return false
                    }
                    input.copyTo(output, 64 * 1024)
                }
            }
            true
        } catch (_: Exception) {
            false
        }
    }
}
