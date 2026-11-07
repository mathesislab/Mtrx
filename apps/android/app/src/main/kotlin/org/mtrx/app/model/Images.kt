package org.mtrx.app.model

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.util.LruCache
import java.util.Collections
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.mtrx.Formula
import uniffi.mtrx.FormulaImage
import uniffi.mtrx.Media
import uniffi.mtrx.MtrxException
import uniffi.mtrx.SessionInterface
import uniffi.mtrx.typesetFormula

/** A formula typeset: black ink on transparent (draw it tinted), and its baseline in pixels from the top. */
class TypesetFormula(val bitmap: Bitmap, val baseline: Int)

/**
 * Avatars, message images and typeset formulas of a session, decoded and
 * kept in memory (the library caches the downloads). Null when an image
 * cannot be loaded; a formula that cannot be typeset is not tried again.
 * [typeset] is the library's (JVM tests, which cannot call it, pass their
 * own).
 */
class Images(
    private val session: SessionInterface,
    private val typeset: suspend (Formula, Float) -> FormulaImage = { formula, emPx -> typesetFormula(formula, emPx) },
) {
    // Created on first use, so that tests can construct an Images.
    private val cache by lazy {
        object : LruCache<String, Bitmap>(CACHE_BYTES) {
            override fun sizeOf(key: String, value: Bitmap): Int = value.byteCount
        }
    }
    private val formulas by lazy {
        object : LruCache<String, TypesetFormula>(FORMULA_BYTES) {
            override fun sizeOf(key: String, value: TypesetFormula): Int = value.bitmap.byteCount
        }
    }
    private val unreadable: MutableSet<String> = Collections.synchronizedSet(mutableSetOf())

    fun cachedAvatar(mxcUri: String, sizePx: Int): Bitmap? = cache.get(avatarKey(mxcUri, sizePx))

    suspend fun avatar(mxcUri: String, sizePx: Int): Bitmap? =
        load(avatarKey(mxcUri, sizePx), sizePx) { session.avatar(mxcUri, sizePx.toUInt()) }

    fun cachedPreview(media: Media): Bitmap? = cache.get(previewKey(media))

    suspend fun preview(media: Media): Bitmap? =
        load(previewKey(media), PREVIEW_PX) { session.mediaPreview(media) }

    /** [formula] at [emPx] pixels per em, if typeset already. */
    fun cachedFormula(formula: Formula, emPx: Int): TypesetFormula? = formulas.get(formulaKey(formula, emPx))

    /** [formula] at [emPx] pixels per em (the body text size); null for LaTeX the library cannot read. */
    suspend fun formula(formula: Formula, emPx: Int): TypesetFormula? {
        val key = formulaKey(formula, emPx)
        formulas.get(key)?.let { return it }
        if (key in unreadable) return null
        val image = try {
            typeset(formula, emPx.toFloat())
        } catch (e: CancellationException) {
            throw e
        } catch (e: MtrxException) {
            unreadable += key
            return null
        }
        val bitmap = withContext(Dispatchers.Default) { BitmapFactory.decodeByteArray(image.png, 0, image.png.size) }
            ?: return null
        return TypesetFormula(bitmap, image.baseline.toInt()).also { formulas.put(key, it) }
    }

    private suspend fun load(key: String, maxPx: Int, fetch: suspend () -> ByteArray): Bitmap? {
        cache.get(key)?.let { return it }
        val bytes = try {
            fetch()
        } catch (e: CancellationException) {
            throw e
        } catch (e: MtrxException) {
            return null
        } catch (e: IllegalStateException) {
            // The session was closed (signed out) while the screen still showed.
            return null
        }
        val bitmap = withContext(Dispatchers.Default) { decode(bytes, maxPx) } ?: return null
        cache.put(key, bitmap)
        return bitmap
    }

    private fun avatarKey(mxcUri: String, sizePx: Int) = "avatar:$sizePx:$mxcUri"

    private fun previewKey(media: Media) = "media:${media.source}"

    private fun formulaKey(formula: Formula, emPx: Int) = "$emPx:${formula.display}:${formula.tex}"

    private companion object {
        /** Larger previews are decoded at a lower resolution. */
        const val PREVIEW_PX = 1280
        val CACHE_BYTES = (Runtime.getRuntime().maxMemory() / 8).coerceAtMost(Int.MAX_VALUE.toLong()).toInt()
        const val FORMULA_BYTES = 16 shl 20

        fun decode(bytes: ByteArray, maxPx: Int): Bitmap? {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
            var sample = 1
            while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= maxPx) sample *= 2
            val options = BitmapFactory.Options().apply { inSampleSize = sample }
            return BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)
        }
    }
}
