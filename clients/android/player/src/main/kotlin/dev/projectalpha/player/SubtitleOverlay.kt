// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.media.MediaFormat
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.Looper
import android.view.Choreographer
import android.view.View
import android.widget.FrameLayout
import androidx.annotation.MainThread
import androidx.annotation.OptIn
import androidx.annotation.VisibleForTesting
import androidx.media3.common.Format
import androidx.media3.common.util.UnstableApi
import androidx.media3.exoplayer.video.VideoFrameMetadataListener
import androidx.media3.ui.SubtitleView
import java.nio.IntBuffer

/**
 * Subtitles above the video [android.view.SurfaceView] (PLAY-AND-006, ADR-0135). Give it the exact size of the video
 * picture (an `AspectRatioFrameLayout` with the surface) and attach it with `Media3Engine.setSubtitleOverlay`.
 *
 * - [cues]: Media3 `SubtitleView` for everything Media3 parses: SRT, WebVTT, TTML, PGS and VobSub bitmaps, and SSA
 *   when libass is not available. The style is the system caption style (Settings › Accessibility › Captions).
 * - [ass]: the libass picture of ASS tracks, in sync with the video frames.
 *
 * Both are ordinary SDR views of the window: above HDR video Android composites them at SDR white, as any UI.
 */
@OptIn(UnstableApi::class)
class SubtitleOverlay(context: Context) : FrameLayout(context) {
    val cues: SubtitleView = SubtitleView(context).apply {
        setUserDefaultStyle()
        setUserDefaultTextSize()
    }
    internal val ass = AssOverlayView(context)

    init {
        addView(cues, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
        addView(ass, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT))
    }
}

/**
 * The libass layer. For every video frame Media3 is about to release (`VideoFrameMetadataListener`, playback thread)
 * the picture for that frame's time is rendered on a render thread and queued with the frame's release time; on each
 * vsync the newest picture whose frame is on screen by the time this draw is presented goes up ([AssMath.due]; the
 * present time comes from the frame timeline on API 33+, else two vsyncs ahead). So the subtitle changes with the
 * frame it belongs to, not with the audio clock.
 *
 * Without video frames (paused, track switched, events that arrived late) the last frame's time is rendered again
 * when [AssSubtitles] reports a change.
 */
@OptIn(UnstableApi::class)
internal class AssOverlayView(context: Context) : View(context) {
    private class Frame(val releaseNs: Long, val timeMs: Long, val bitmap: Bitmap?, val x: Int, val y: Int, val unchanged: Boolean)

    private val main = Handler(Looper.getMainLooper())
    private var thread: HandlerThread? = null
    private var render: Handler? = null
    @Volatile private var subtitles: AssSubtitles? = null

    // Render thread.
    private var lastPtsMs = Long.MIN_VALUE
    private var videoW = 0
    private var videoH = 0
    @Volatile private var viewW = 0
    @Volatile private var viewH = 0

    // Guarded by queue.
    private val queue = ArrayDeque<Frame>()

    // Main thread.
    private var shown: Frame? = null
    private var vsyncScheduled = false
    private var shownTimeMs = Long.MIN_VALUE

    /** Media time of the picture on screen, for tests. Main thread. */
    @VisibleForTesting
    internal val shownMediaTimeMs: Long get() = shownTimeMs

    /** A libass picture is up. Main thread. */
    @VisibleForTesting
    internal val showsPicture: Boolean get() = shown?.bitmap != null

    val frameListener = VideoFrameMetadataListener { presentationTimeUs, releaseTimeNs, format, _: MediaFormat? ->
        render?.post { renderFor(presentationTimeUs / 1000, releaseTimeNs, format) }
    }

    @MainThread
    fun attach(subtitles: AssSubtitles?) {
        detach()
        this.subtitles = subtitles ?: return
        val t = HandlerThread("pa-ass-render").also { it.start() }
        thread = t
        val h = Handler(t.looper)
        render = h
        subtitles.onChanged = { h.post { rerender() } }
    }

    @MainThread
    fun detach() {
        subtitles?.onChanged = null
        subtitles = null
        thread?.quitSafely()
        thread = null
        render = null
        synchronized(queue) { queue.clear() }
        shown = null
        shownTimeMs = Long.MIN_VALUE
        invalidate()
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        viewW = w
        viewH = h
        render?.post { rerender() }
    }

    override fun onDetachedFromWindow() {
        super.onDetachedFromWindow()
        vsyncScheduled = false
    }

    override fun onDraw(canvas: Canvas) {
        val f = shown ?: return
        f.bitmap?.let { canvas.drawBitmap(it, f.x.toFloat(), f.y.toFloat(), null) }
    }

    // ---- Render thread ----------------------------------------------------------------------------------------

    private fun renderFor(ptsMs: Long, releaseNs: Long, format: Format) {
        val w = if (format.pixelWidthHeightRatio > 0f) (format.width * format.pixelWidthHeightRatio).toInt() else format.width
        videoW = w
        videoH = format.height
        lastPtsMs = ptsMs
        produce(ptsMs, releaseNs)
    }

    private fun rerender() {
        if (lastPtsMs != Long.MIN_VALUE) produce(lastPtsMs, System.nanoTime())
    }

    private fun produce(timeMs: Long, releaseNs: Long) {
        val s = subtitles ?: return
        val frame = when (val r = s.render(timeMs, viewW, viewH, videoW, videoH)) {
            AssSubtitles.Result.Unchanged -> Frame(releaseNs, timeMs, null, 0, 0, unchanged = true)
            AssSubtitles.Result.Empty -> Frame(releaseNs, timeMs, null, 0, 0, unchanged = false)
            is AssSubtitles.Result.Changed -> {
                val img = r.image
                val bitmap = Bitmap.createBitmap(img.width, img.height, Bitmap.Config.ARGB_8888)
                bitmap.copyPixelsFromBuffer(IntBuffer.wrap(img.pixels))
                Frame(releaseNs, timeMs, bitmap, img.x, img.y, unchanged = false)
            }
        }
        synchronized(queue) {
            queue.addLast(frame)
            // A stalled UI must not pile up pictures.
            while (queue.size > MAX_QUEUE) queue.removeFirst()
        }
        main.post(::scheduleVsync)
    }

    // ---- Main thread --------------------------------------------------------------------------------------------

    private fun scheduleVsync() {
        if (vsyncScheduled || subtitles == null) return
        vsyncScheduled = true
        if (Build.VERSION.SDK_INT >= 33) {
            Choreographer.getInstance().postVsyncCallback { data ->
                onVsync(data.preferredFrameTimeline.expectedPresentationTimeNanos)
            }
        } else {
            Choreographer.getInstance().postFrameCallback { frameTimeNanos ->
                val periodNs = (1_000_000_000.0 / (display?.refreshRate?.takeIf { it > 1f } ?: 60f)).toLong()
                onVsync(AssMath.presentTime(frameTimeNanos, periodNs))
            }
        }
    }

    private fun onVsync(presentNs: Long) {
        vsyncScheduled = false
        var next: Frame? = null
        var pending: Boolean
        synchronized(queue) {
            val due = AssMath.due(queue.map { it.releaseNs }, presentNs)
            for (i in 0..due) {
                val f = queue.removeFirst()
                if (!f.unchanged) next = f
                shownTimeMs = f.timeMs
            }
            pending = queue.isNotEmpty()
        }
        next?.let {
            shown = it
            invalidate()
        }
        if (pending) scheduleVsync()
    }

    private companion object {
        const val MAX_QUEUE = 8
    }
}
