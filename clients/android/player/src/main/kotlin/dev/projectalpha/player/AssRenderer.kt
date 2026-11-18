// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.Format
import androidx.media3.common.MimeTypes
import androidx.media3.common.util.UnstableApi
import androidx.media3.decoder.DecoderInputBuffer
import androidx.media3.exoplayer.BaseRenderer
import androidx.media3.exoplayer.FormatHolder
import androidx.media3.exoplayer.RendererCapabilities
import androidx.media3.exoplayer.source.MediaSource
import androidx.media3.common.util.Consumer
import androidx.media3.extractor.text.CuesWithTiming
import androidx.media3.extractor.text.DefaultSubtitleParserFactory
import androidx.media3.extractor.text.SubtitleParser

/**
 * The text renderer of SSA/ASS tracks (PLAY-AND-006, ADR-0135): Media3 parses subtitles while extracting and turns
 * SSA into plain cues (no karaoke, rotation, drawings or fonts), so for ASS we keep the raw samples ([parserFactory]
 * declines SSA) and this renderer, placed before Media3's `TextRenderer`, hands them to libass ([AssSubtitles]) with
 * their media times. It draws nothing itself: [AssOverlayView] renders libass at the time of each video frame.
 *
 * Embedded tracks (MKV `S_TEXT/ASS`) arrive as Matroska samples with the header in `initializationData[1]`. For an
 * external .ass/.ssa Media3 gets a parser that yields no cues ([parserFactory]): the track exists, can be selected
 * and seeked in, but has no samples; the file itself comes from [AssSubtitles.fetchExternal] by its `ext-N` id.
 */
@OptIn(UnstableApi::class)
internal class AssRenderer(private val subtitles: AssSubtitles) : BaseRenderer(C.TRACK_TYPE_TEXT) {
    private val formatHolder = FormatHolder()
    private val buffer = DecoderInputBuffer(DecoderInputBuffer.BUFFER_REPLACEMENT_MODE_NORMAL)
    private var sample = ByteArray(4096)
    private var inputEnded = false
    private var pendingExternal: Format? = null

    override fun getName(): String = NAME

    override fun supportsFormat(format: Format): Int = RendererCapabilities.create(
        when {
            isAss(format) -> if (format.cryptoType == C.CRYPTO_TYPE_NONE) C.FORMAT_HANDLED else C.FORMAT_UNSUPPORTED_DRM
            MimeTypes.isText(format.sampleMimeType) -> C.FORMAT_UNSUPPORTED_SUBTYPE
            else -> C.FORMAT_UNSUPPORTED_TYPE
        },
    )

    override fun onStreamChanged(
        formats: Array<out Format>,
        startPositionUs: Long,
        offsetUs: Long,
        mediaPeriodId: MediaSource.MediaPeriodId,
    ) {
        inputEnded = false
        start(formats[0])
    }

    override fun onPositionReset(positionUs: Long, joining: Boolean, sampleStreamIsResetToKeyFrame: Boolean) {
        // libass keeps the events it has (duplicates are dropped by ReadOrder); samples after a seek just add more.
        inputEnded = false
    }

    override fun onDisabled() {
        pendingExternal = null
        subtitles.stop()
    }

    override fun render(positionUs: Long, elapsedRealtimeUs: Long) {
        pendingExternal?.let { if (subtitles.startExternal(externalId(it), it.language)) pendingExternal = null }
        while (!inputEnded) {
            buffer.clear()
            when (readSource(formatHolder, buffer, 0)) {
                C.RESULT_FORMAT_READ -> formatHolder.format?.let(::start)
                C.RESULT_BUFFER_READ -> if (buffer.isEndOfStream) inputEnded = true else feed()
                else -> return
            }
        }
    }

    override fun isReady(): Boolean = true

    override fun isEnded(): Boolean = inputEnded

    private fun start(format: Format) {
        if (format.initializationData.size >= 2) {
            pendingExternal = null
            subtitles.startEmbedded(format.initializationData[1], format.language)
        } else {
            // An external file: its track has no samples. Started once its bytes are there.
            pendingExternal = format
            if (subtitles.startExternal(externalId(format), format.language)) pendingExternal = null
        }
    }

    private fun feed() {
        val data = buffer.data ?: return
        data.flip()
        val size = data.remaining()
        if (sample.size < size) sample = ByteArray(size)
        data.get(sample, 0, size)
        val mediaTimeUs = buffer.timeUs - streamOffsetUs
        subtitles.addSample(sample, size, mediaTimeUs / 1000)
    }

    companion object {
        const val NAME = "AssRenderer"

        /** SSA/ASS: raw Matroska samples, or an external file that [parserFactory] turned into an empty cue track. */
        fun isAss(format: Format): Boolean =
            format.sampleMimeType == MimeTypes.TEXT_SSA ||
                (format.sampleMimeType == MimeTypes.APPLICATION_MEDIA3_CUES && format.codecs == MimeTypes.TEXT_SSA)

        /** `ext-N` of an external subtitle; Media3 prefixes ids of merged sources with `<source>:`. */
        fun externalId(format: Format): String = format.id?.replace(Regex("^\\d+:"), "").orEmpty()

        /**
         * Media3's subtitle parsers, except SSA: an embedded track (it has the Matroska header in
         * `initializationData`) is declined and stays raw for [AssRenderer]; a side-loaded file (no header) gets a
         * parser without cues, so Media3 still has a seekable track for it (an unparsed side-loaded track cannot be
         * seeked in: `MergingMediaPeriod` fails).
         */
        fun parserFactory(delegate: SubtitleParser.Factory = DefaultSubtitleParserFactory()): SubtitleParser.Factory =
            object : SubtitleParser.Factory {
                override fun supportsFormat(format: Format): Boolean =
                    if (format.sampleMimeType == MimeTypes.TEXT_SSA) format.initializationData.isEmpty() else delegate.supportsFormat(format)

                override fun getCueReplacementBehavior(format: Format): Int =
                    if (format.sampleMimeType == MimeTypes.TEXT_SSA) Format.CUE_REPLACEMENT_BEHAVIOR_MERGE else delegate.getCueReplacementBehavior(format)

                override fun create(format: Format): SubtitleParser =
                    if (format.sampleMimeType == MimeTypes.TEXT_SSA) NoCues else delegate.create(format)
            }

        private object NoCues : SubtitleParser {
            override fun parse(
                data: ByteArray,
                offset: Int,
                length: Int,
                outputOptions: SubtitleParser.OutputOptions,
                output: Consumer<CuesWithTiming>,
            ) = Unit

            override fun getCueReplacementBehavior(): Int = Format.CUE_REPLACEMENT_BEHAVIOR_MERGE
        }
    }
}
