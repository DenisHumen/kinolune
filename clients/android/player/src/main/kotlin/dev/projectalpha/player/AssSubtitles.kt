// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.net.Uri
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.PlaybackException
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSourceException
import androidx.media3.datasource.DataSourceUtil
import androidx.media3.datasource.DataSpec
import com.sun.jna.NativeLong
import com.sun.jna.Pointer
import java.io.File
import java.util.concurrent.Executors

/**
 * The libass side of one player (PLAY-AND-006, ADR-0135): the selected ASS track, its fonts and rendering.
 *
 * - [AssRenderer] (playback thread) starts and stops the track and feeds it events with their media times.
 * - [AssOverlayView] (its render thread) calls [render] for the time of each video frame.
 * - Fonts of the MKV and external .ass files are read on a background thread ([loadAttachments], [fetchExternal]).
 *
 * All libass calls hold one lock. [onChanged] is called (on any thread) when a frame already rendered may be stale:
 * a track started or stopped, fonts arrived, an event covering the last rendered time came in after it.
 */
@OptIn(UnstableApi::class)
internal class AssSubtitles(private val systemFonts: Map<String, File> = SubtitleFonts.system()) : AutoCloseable {
    /** A rendered frame: premultiplied `0xAABBGGRR` [pixels] of the box ([x], [y], [width], [height]) of the frame. */
    class Image(val x: Int, val y: Int, val width: Int, val height: Int, val pixels: IntArray)

    sealed interface Result {
        /** Same picture as the last result. */
        data object Unchanged : Result

        /** Nothing to show. */
        data object Empty : Result

        class Changed(val image: Image) : Result
    }

    @Volatile var onChanged: (() -> Unit)? = null

    private val lock = Any()
    private val io = Executors.newSingleThreadExecutor { Thread(it, "pa-ass-io").apply { isDaemon = true } }

    // Guarded by lock.
    private var library: Pointer? = null
    private var renderer: Pointer? = null
    private var track: Pointer? = null
    private var family: String? = null
    private var fontsDirty = true
    private var forceNext = true
    private var lastShown = false
    private var lastTimeMs = Long.MIN_VALUE
    private var frameW = 0
    private var frameH = 0
    private var storageW = 0
    private var storageH = 0
    private var closed = false
    private val external = HashMap<String, ByteArray>()
    private val attachmentsLoaded = HashSet<String>()

    /** The selected track is active (an embedded or external ASS track is being rendered). */
    val active: Boolean get() = synchronized(lock) { track != null }

    // ---- Track (playback thread) ----------------------------------------------------------------------------

    /** An embedded track: [header] is the codec private data (`[Script Info]` … `[Events]` Format line). */
    fun startEmbedded(header: ByteArray?, language: String?) {
        synchronized(lock) {
            val lib = library() ?: return
            dropTrack()
            val t = LibAss.ass_new_track(lib) ?: return
            if (header != null && header.isNotEmpty()) LibAss.ass_process_codec_private(t, header, header.size)
            track = t
            setFamily(language)
        }
        changed()
    }

    /** An external file added with `addExternalSubtitle`: false until its bytes are fetched ([fetchExternal]). */
    fun startExternal(id: String, language: String?): Boolean {
        synchronized(lock) {
            val bytes = external[id] ?: return false
            val lib = library() ?: return true
            dropTrack()
            track = LibAss.ass_read_memory(lib, bytes.copyOf(), NativeLong(bytes.size.toLong()), null)
            if (track == null) Log.w(TAG, "libass could not read external subtitle $id")
            setFamily(language)
        }
        changed()
        return true
    }

    fun stop() {
        val had = synchronized(lock) {
            val had = track != null
            dropTrack()
            had
        }
        if (had) changed()
    }

    /** One Matroska sample of the track at [startMs] of the media (see [AssMath.chunk]). */
    fun addSample(data: ByteArray, size: Int, startMs: Long) {
        val chunk = AssMath.chunk(data, size) ?: return
        var covers: Boolean
        synchronized(lock) {
            val t = track ?: return
            val event = data.copyOfRange(chunk.offset, size)
            LibAss.ass_process_chunk(t, event, event.size, startMs, chunk.durationMs)
            covers = lastTimeMs in startMs until startMs + chunk.durationMs
            if (covers) forceNext = true
        }
        if (covers) changed()
    }

    // ---- Background IO ----------------------------------------------------------------------------------------

    /** Reads an external .ass/.ssa file for [startExternal]. */
    fun fetchExternal(id: String, uri: String, dataSources: DataSource.Factory) {
        io.execute {
            val bytes = try {
                readAll(dataSources, uri)
            } catch (e: Exception) {
                Log.w(TAG, "external subtitle $uri: ${e.message}")
                return@execute
            }
            synchronized(lock) { if (!closed) external[id] = bytes }
            changed()
        }
    }

    /** Adds the fonts attached to the MKV at [uri] (once per file). */
    fun loadAttachments(uri: String, dataSources: DataSource.Factory) {
        synchronized(lock) { if (closed || !attachmentsLoaded.add(uri)) return }
        io.execute {
            val fonts = try {
                MkvAttachments.fonts { position, length -> readRange(dataSources, uri, position, length) }
            } catch (e: Exception) {
                Log.w(TAG, "attachments of $uri: ${e.message}")
                return@execute
            }
            if (fonts.isEmpty()) return@execute
            synchronized(lock) {
                val lib = library() ?: return@execute
                for (f in fonts) LibAss.ass_add_font(lib, f.name, f.data, f.data.size)
                fontsDirty = true
                forceNext = true
            }
            Log.i(TAG, "${fonts.size} fonts attached to the file")
            changed()
        }
    }

    // ---- Rendering (render thread) ------------------------------------------------------------------------------

    /**
     * The picture at [timeMs] of the media for a [width]×[height] px overlay over video of [videoWidth]×[videoHeight]
     * (libass storage size: ScaledBorderAndShadow, blur and PlayRes scaling follow the video).
     */
    fun render(timeMs: Long, width: Int, height: Int, videoWidth: Int, videoHeight: Int): Result {
        synchronized(lock) {
            val t = track
            val r = renderer
            if (t == null || r == null || width <= 0 || height <= 0) {
                lastTimeMs = timeMs
                if (!lastShown) return Result.Unchanged
                lastShown = false
                return Result.Empty
            }
            if (width != frameW || height != frameH) {
                LibAss.ass_set_frame_size(r, width, height)
                frameW = width
                frameH = height
                forceNext = true
            }
            val sw = if (videoWidth > 0) videoWidth else width
            val sh = if (videoHeight > 0) videoHeight else height
            if (sw != storageW || sh != storageH) {
                LibAss.ass_set_storage_size(r, sw, sh)
                storageW = sw
                storageH = sh
                forceNext = true
            }
            if (fontsDirty) {
                LibAss.ass_set_fonts(r, null, family ?: "Roboto", LibAss.FONTPROVIDER_NONE, null, 1)
                fontsDirty = false
                forceNext = true
            }
            val change = IntArray(1)
            val first = LibAss.ass_render_frame(r, t, timeMs, change)
            lastTimeMs = timeMs
            if (change[0] == 0 && !forceNext) return Result.Unchanged
            forceNext = false
            if (first == null) {
                if (!lastShown) return Result.Unchanged
                lastShown = false
                return Result.Empty
            }
            val image = compose(first, width, height) ?: run {
                lastShown = false
                return Result.Empty
            }
            lastShown = true
            return Result.Changed(image)
        }
    }

    override fun close() {
        synchronized(lock) {
            closed = true
            dropTrack()
            renderer?.let { LibAss.ass_renderer_done(it) }
            library?.let { LibAss.ass_library_done(it) }
            renderer = null
            library = null
            external.clear()
        }
        io.shutdownNow()
    }

    // ---- Internals (lock held) ------------------------------------------------------------------------------------

    private fun library(): Pointer? {
        if (closed) return null
        library?.let { return it }
        val lib = LibAss.ass_library_init() ?: return null
        LibAss.ass_set_extract_fonts(lib, 0)
        // The system fonts are added as memory fonts once: a fonts dir would be read again on every ass_set_fonts.
        for ((name, file) in systemFonts) {
            try {
                val data = file.readBytes()
                LibAss.ass_add_font(lib, name, data, data.size)
            } catch (e: Exception) {
                Log.w(TAG, "font $file: ${e.message}")
            }
        }
        val r = LibAss.ass_renderer_init(lib)
        if (r == null) {
            LibAss.ass_library_done(lib)
            return null
        }
        library = lib
        renderer = r
        fontsDirty = true
        return lib
    }

    private fun setFamily(language: String?) {
        val f = SubtitleFonts.fallbackFamily(language, systemFonts.keys)
        if (f != family) {
            family = f
            fontsDirty = true
        }
        forceNext = true
    }

    private fun dropTrack() {
        track?.let { LibAss.ass_free_track(it) }
        track = null
        forceNext = true
    }

    private fun changed() {
        onChanged?.invoke()
    }

    /** Blends the image list into the smallest box that holds it. */
    private fun compose(first: Pointer, width: Int, height: Int): Image? {
        var minX = Int.MAX_VALUE
        var minY = Int.MAX_VALUE
        var maxX = Int.MIN_VALUE
        var maxY = Int.MIN_VALUE
        var p: Pointer? = first
        while (p != null) {
            val w = p.getInt(LibAss.Image.W)
            val h = p.getInt(LibAss.Image.H)
            if (w > 0 && h > 0) {
                val x = p.getInt(LibAss.Image.DST_X)
                val y = p.getInt(LibAss.Image.DST_Y)
                minX = minOf(minX, x)
                minY = minOf(minY, y)
                maxX = maxOf(maxX, x + w)
                maxY = maxOf(maxY, y + h)
            }
            p = p.getPointer(LibAss.Image.NEXT)
        }
        minX = minX.coerceAtLeast(0)
        minY = minY.coerceAtLeast(0)
        maxX = maxX.coerceAtMost(width)
        maxY = maxY.coerceAtMost(height)
        if (maxX <= minX || maxY <= minY) return null
        val boxW = maxX - minX
        val boxH = maxY - minY
        val pixels = IntArray(boxW * boxH)
        var coverage = ByteArray(0)
        p = first
        while (p != null) {
            val w = p.getInt(LibAss.Image.W)
            val h = p.getInt(LibAss.Image.H)
            if (w > 0 && h > 0) {
                val stride = p.getInt(LibAss.Image.STRIDE)
                val size = stride * (h - 1) + w
                if (coverage.size < size) coverage = ByteArray(size)
                p.getPointer(LibAss.Image.BITMAP)?.read(0, coverage, 0, size)
                AssMath.blend(
                    pixels, boxW, minX, minY, coverage, w, h, stride,
                    p.getInt(LibAss.Image.COLOR),
                    p.getInt(LibAss.Image.DST_X), p.getInt(LibAss.Image.DST_Y),
                )
            }
            p = p.getPointer(LibAss.Image.NEXT)
        }
        return Image(minX, minY, boxW, boxH, pixels)
    }

    private companion object {
        const val TAG = "AssSubtitles"
        const val MAX_EXTERNAL_BYTES = 32 * 1024 * 1024

        fun readAll(dataSources: DataSource.Factory, uri: String): ByteArray {
            val source = dataSources.createDataSource()
            try {
                source.open(DataSpec(Uri.parse(uri)))
                val bytes = DataSourceUtil.readToEnd(source)
                require(bytes.size <= MAX_EXTERNAL_BYTES) { "too large" }
                return bytes
            } finally {
                source.close()
            }
        }

        /** Up to [length] bytes at [position]; fewer at the end of the file. */
        fun readRange(dataSources: DataSource.Factory, uri: String, position: Long, length: Int): ByteArray {
            val source = dataSources.createDataSource()
            try {
                // Open-ended: a bounded range past the end of the file is an error for some sources.
                source.open(DataSpec.Builder().setUri(Uri.parse(uri)).setPosition(position).build())
                val out = ByteArray(length)
                var n = 0
                while (n < length) {
                    val read = source.read(out, n, length - n)
                    if (read == C.RESULT_END_OF_INPUT) break
                    n += read
                }
                return if (n == length) out else out.copyOf(n)
            } catch (e: DataSourceException) {
                if (e.reason == PlaybackException.ERROR_CODE_IO_READ_POSITION_OUT_OF_RANGE) return ByteArray(0)
                throw e
            } finally {
                source.close()
            }
        }
    }
}
