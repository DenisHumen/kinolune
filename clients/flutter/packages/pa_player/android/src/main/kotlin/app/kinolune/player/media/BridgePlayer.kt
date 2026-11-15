// SPDX-License-Identifier: MPL-2.0
package app.kinolune.player.media

import android.net.Uri
import android.os.Looper
import android.os.SystemClock
import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.common.SimpleBasePlayer
import androidx.media3.common.util.UnstableApi
import com.google.common.util.concurrent.Futures
import com.google.common.util.concurrent.ListenableFuture
import java.io.File

/**
 * A Media3 [Player] that plays nothing itself (PLAY-AND-008, ADR-NNNN): it mirrors the state of our libmpv
 * engine, which the Dart player reports ([update]), and turns what the session asks for (notification, lock
 * screen, Bluetooth and wired headset buttons, Android Auto / Wear later) into commands for Dart ([onCommand]:
 * `play`, `pause`, `seek` with `positionMs`, `next`, `previous`, `stop`).
 *
 * The playlist is a window of up to three items, `[previous] current [next]`, so that Media3 offers
 * previous / next exactly when the Dart queue has them; the neighbours are placeholders.
 */
@OptIn(UnstableApi::class)
class BridgePlayer(
    looper: Looper,
    private val onCommand: (String, Map<String, Any?>) -> Unit,
) : SimpleBasePlayer(looper) {
    var snapshot: SessionSnapshot = SessionSnapshot.EMPTY
        private set

    /** New state from Dart (on the main thread). */
    fun update(s: SessionSnapshot) {
        snapshot = s
        invalidateState()
    }

    private val currentIndex: Int get() = if (snapshot.hasPrevious) 1 else 0

    override fun getState(): State {
        val s = snapshot
        val b = State.Builder()
            .setAvailableCommands(commandsFor(s))
            .setSeekBackIncrementMs(SEEK_STEP_MS)
            .setSeekForwardIncrementMs(SEEK_STEP_MS)
            .setMaxSeekToPreviousPositionMs(MAX_SEEK_TO_PREVIOUS_MS)
        if (s.isEmpty) {
            return b.setPlaybackState(STATE_IDLE)
                .setPlayWhenReady(false, PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST)
                .build()
        }
        val items = ArrayList<MediaItemData>(3)
        if (s.hasPrevious) items.add(placeholder("previous:${s.id}"))
        items.add(current(s))
        if (s.hasNext) items.add(placeholder("next:${s.id}"))
        return b.setPlaylist(items)
            .setCurrentMediaItemIndex(if (s.hasPrevious) 1 else 0)
            .setPlayWhenReady(s.playing, PLAY_WHEN_READY_CHANGE_REASON_USER_REQUEST)
            .setPlaybackState(
                when {
                    s.ended -> STATE_ENDED
                    s.buffering -> STATE_BUFFERING
                    else -> STATE_READY
                },
            )
            .setIsLoading(s.buffering && !s.ended)
            .setPlaybackParameters(PlaybackParameters(s.rate))
            .setContentPositionMs { s.positionAt(SystemClock.elapsedRealtime()) }
            .build()
    }

    private fun commandsFor(s: SessionSnapshot): Player.Commands {
        val b = Player.Commands.Builder().addAll(
            COMMAND_PLAY_PAUSE,
            COMMAND_STOP,
            COMMAND_GET_CURRENT_MEDIA_ITEM,
            COMMAND_GET_TIMELINE,
            COMMAND_GET_METADATA,
        )
        if (s.isEmpty) return b.build()
        if (s.seekable) {
            b.addAll(
                COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM,
                COMMAND_SEEK_TO_DEFAULT_POSITION,
                COMMAND_SEEK_TO_PREVIOUS,
                COMMAND_SEEK_BACK,
                COMMAND_SEEK_FORWARD,
            )
        }
        if (s.hasPrevious) b.addAll(COMMAND_SEEK_TO_PREVIOUS, COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
        if (s.hasNext) b.addAll(COMMAND_SEEK_TO_NEXT, COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
        return b.build()
    }

    private fun metadata(s: SessionSnapshot): MediaMetadata = MediaMetadata.Builder()
        .setTitle(s.title)
        .setDisplayTitle(s.title)
        .setArtist(s.subtitle)
        .setSubtitle(s.subtitle)
        .setAlbumTitle(s.album)
        .setArtworkUri(artworkUri(s.artworkUri))
        .setMediaType(
            when (s.kind) {
                "music" -> MediaMetadata.MEDIA_TYPE_MUSIC
                "live" -> MediaMetadata.MEDIA_TYPE_VIDEO
                else -> MediaMetadata.MEDIA_TYPE_MOVIE
            },
        )
        .setIsPlayable(true)
        .setIsBrowsable(false)
        .build()

    private fun current(s: SessionSnapshot): MediaItemData {
        val md = metadata(s)
        val item = MediaItem.Builder().setMediaId(s.id).setMediaMetadata(md).build()
        val b = MediaItemData.Builder("current:${s.id}")
            .setMediaItem(item)
            .setMediaMetadata(md)
            .setIsSeekable(s.seekable)
            .setDurationUs(if (s.durationMs > 0 && !s.isLive) s.durationMs * 1000 else C.TIME_UNSET)
        if (s.isLive) {
            b.setIsDynamic(true).setLiveConfiguration(MediaItem.LiveConfiguration.Builder().build())
        }
        return b.build()
    }

    private fun placeholder(uid: String): MediaItemData =
        MediaItemData.Builder(uid).setIsPlaceholder(true).build()

    private fun send(action: String, positionMs: Long? = null) {
        onCommand(action, if (positionMs == null) emptyMap() else mapOf("positionMs" to positionMs))
    }

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
        val s = snapshot
        // Optimistic: the session shows the new state at once; Dart confirms it.
        val now = SystemClock.elapsedRealtime()
        snapshot = s.copy(playing = playWhenReady, positionMs = s.positionAt(now), receivedAtMs = now)
        send(if (playWhenReady) "play" else "pause")
        return Futures.immediateVoidFuture()
    }

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> {
        val cur = currentIndex
        when {
            seekCommand == COMMAND_SEEK_TO_NEXT || seekCommand == COMMAND_SEEK_TO_NEXT_MEDIA_ITEM ||
                (mediaItemIndex > cur && mediaItemIndex != C.INDEX_UNSET) -> send("next")
            (seekCommand == COMMAND_SEEK_TO_PREVIOUS || seekCommand == COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM) &&
                mediaItemIndex < cur -> send("previous")
            mediaItemIndex in 0 until cur -> send("previous")
            else -> {
                val target = if (positionMs == C.TIME_UNSET) 0 else positionMs.coerceAtLeast(0)
                snapshot = snapshot.copy(positionMs = target, receivedAtMs = SystemClock.elapsedRealtime())
                send("seek", target)
            }
        }
        return Futures.immediateVoidFuture()
    }

    override fun handleStop(): ListenableFuture<*> {
        val s = snapshot
        snapshot = s.copy(playing = false)
        send("stop")
        return Futures.immediateVoidFuture()
    }

    companion object {
        private const val SEEK_STEP_MS = 10_000L
        private const val MAX_SEEK_TO_PREVIOUS_MS = 3_000L

        /** Media3 loads `file://`, `content://` and http(s); a bare path becomes `file://`. */
        fun artworkUri(raw: String?): Uri? {
            if (raw.isNullOrEmpty()) return null
            if (raw.startsWith("/")) return Uri.fromFile(File(raw))
            return Uri.parse(raw)
        }
    }
}
