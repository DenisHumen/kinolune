// SPDX-License-Identifier: MPL-2.0
package dev.projectalpha.player

import android.content.Context
import android.net.Uri
import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.PlaybackException
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.BaseDataSource
import androidx.media3.datasource.DataSource
import androidx.media3.datasource.DataSourceException
import androidx.media3.datasource.DataSpec
import androidx.media3.datasource.DefaultDataSource
import androidx.media3.datasource.TransferListener
import dev.projectalpha.core.CoreByteSource
import dev.projectalpha.core.StreamException
import dev.projectalpha.core.streamOpen
import java.util.concurrent.atomic.AtomicLong

/**
 * Media3 `DataSource` over the core's `ByteSource` (PLAY-AND-003, ADR-0127).
 *
 * The app hands players the URL of the core's loopback proxy (`http://127.0.0.1:<port>/<token>/<source>/<path>`,
 * ADR-0038) for files of SMB, WebDAV, NFS and other sources. For such a URL this source first asks the core
 * through UniFFI (`streamOpen`): when the proxy runs in the same core library, the file is read with positional
 * reads straight from the source backend, without the socket and HTTP. Otherwise (the URL is not ours, the core
 * library is not loaded, [direct] is off) it falls back to [upstream], which reads the same URL over HTTP through
 * the proxy. Every other URI (`file://`, `content://`, assets, remote http) goes to [upstream] unchanged.
 *
 * Reads go to the core in chunks of [CHUNK_BYTES]: a JNA call per extractor read would cost more than the read.
 */
@OptIn(UnstableApi::class)
class CoreDataSource(
    private val upstream: DataSource,
    private val direct: Boolean = true,
    private val counters: Counters? = null,
    private val opener: (String) -> CoreByteSource = ::streamOpen,
) : DataSource {

    /** How the sources of one factory opened and what they read: for stats and tests. */
    class Counters {
        val directOpens = AtomicLong()
        val fallbackOpens = AtomicLong()
        val directBytes = AtomicLong()
    }

    /** Creates [CoreDataSource]s over [upstream] (by default Media3's [DefaultDataSource]: files, content, HTTP). */
    class Factory(
        context: Context,
        private val upstream: DataSource.Factory = DefaultDataSource.Factory(context),
        private val direct: Boolean = true,
    ) : DataSource.Factory {
        val counters = Counters()

        override fun createDataSource(): DataSource = CoreDataSource(upstream.createDataSource(), direct, counters)
    }

    private val core = CoreReader(counters)
    private var current: DataSource? = null

    override fun addTransferListener(transferListener: TransferListener) {
        core.addTransferListener(transferListener)
        upstream.addTransferListener(transferListener)
    }

    override fun open(dataSpec: DataSpec): Long {
        val source = if (direct && isLoopbackHttp(dataSpec.uri)) openCore(dataSpec.uri.toString()) else null
        if (source == null) {
            counters?.fallbackOpens?.incrementAndGet()
            current = upstream
            return upstream.open(dataSpec)
        }
        counters?.directOpens?.incrementAndGet()
        current = core
        return core.open(dataSpec, source)
    }

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int =
        (current ?: throw DataSourceException(PlaybackException.ERROR_CODE_IO_UNSPECIFIED)).read(buffer, offset, length)

    override fun getUri(): Uri? = current?.uri

    override fun getResponseHeaders(): Map<String, List<String>> = current?.responseHeaders ?: emptyMap()

    override fun close() {
        val c = current
        current = null
        c?.close()
    }

    /** The core's byte source for [url], or `null` when the core does not serve it here (read over HTTP then). */
    private fun openCore(url: String): CoreByteSource? {
        if (coreMissing) return null
        return try {
            opener(url)
        } catch (_: StreamException.NotLocal) {
            null
        } catch (e: StreamException) {
            throw ioError(e)
        } catch (_: LinkageError) {
            // No core library in this process (UnsatisfiedLinkError, NoClassDefFoundError): the proxy still works.
            coreMissing = true
            null
        }
    }

    companion object {
        /** Bytes per core read: large enough to amortise the FFI call, small enough to stop a load quickly. */
        const val CHUNK_BYTES = 512 * 1024

        @Volatile private var coreMissing = false

        /** `http://127.0.0.1:<port>/…` or `http://localhost:<port>/…`: what the core's proxy hands out. */
        fun isLoopbackHttp(uri: Uri): Boolean =
            uri.scheme.equals("http", ignoreCase = true) &&
                (uri.host == "127.0.0.1" || uri.host.equals("localhost", ignoreCase = true)) &&
                uri.port > 0

        /** Media3 error codes for core stream errors; Media3Mapping turns every `2xxx` into `EngineError.Source`. */
        fun errorCode(e: StreamException): Int = when (e) {
            is StreamException.NotFound -> PlaybackException.ERROR_CODE_IO_FILE_NOT_FOUND
            is StreamException.Denied -> PlaybackException.ERROR_CODE_IO_NO_PERMISSION
            is StreamException.Unavailable -> PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED
            is StreamException.TimedOut -> PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_TIMEOUT
            else -> PlaybackException.ERROR_CODE_IO_UNSPECIFIED
        }

        internal fun ioError(e: StreamException) = DataSourceException(e.message, e, errorCode(e))
    }
}

/** Reads one opened [CoreByteSource] in chunks of [CoreDataSource.CHUNK_BYTES]; Media3 transfer events as a network source. */
@OptIn(UnstableApi::class)
private class CoreReader(private val counters: CoreDataSource.Counters?) : BaseDataSource(/* isNetwork = */ true) {
    private var source: CoreByteSource? = null
    private var uri: Uri? = null
    private var position = 0L
    private var remaining = 0L
    private var opened = false

    private var chunk = ByteArray(0)
    private var chunkStart = 0L
    private var chunkLength = 0

    fun open(dataSpec: DataSpec, core: CoreByteSource): Long {
        uri = dataSpec.uri
        source = core
        transferInitializing(dataSpec)
        val size = core.size().toLong()
        if (dataSpec.position > size) {
            close()
            throw DataSourceException(PlaybackException.ERROR_CODE_IO_READ_POSITION_OUT_OF_RANGE)
        }
        position = dataSpec.position
        val left = size - position
        remaining = if (dataSpec.length == C.LENGTH_UNSET.toLong()) left else minOf(dataSpec.length, left)
        chunkLength = 0
        opened = true
        transferStarted(dataSpec)
        return remaining
    }

    override fun open(dataSpec: DataSpec): Long = throw UnsupportedOperationException("opened by CoreDataSource")

    override fun read(buffer: ByteArray, offset: Int, length: Int): Int {
        if (length == 0) return 0
        if (remaining == 0L) return C.RESULT_END_OF_INPUT
        val core = source ?: throw DataSourceException(PlaybackException.ERROR_CODE_IO_UNSPECIFIED)
        if (position < chunkStart || position >= chunkStart + chunkLength) {
            val want = minOf(remaining, CoreDataSource.CHUNK_BYTES.toLong()).toInt()
            chunk = try {
                core.readAt(position.toULong(), want.toUInt())
            } catch (e: StreamException) {
                throw CoreDataSource.ioError(e)
            }
            if (chunk.isEmpty()) {
                throw DataSourceException(PlaybackException.ERROR_CODE_IO_READ_POSITION_OUT_OF_RANGE)
            }
            chunkStart = position
            chunkLength = chunk.size
        }
        val inChunk = (position - chunkStart).toInt()
        val n = minOf(length.toLong(), remaining, (chunkLength - inChunk).toLong()).toInt()
        System.arraycopy(chunk, inChunk, buffer, offset, n)
        position += n
        remaining -= n
        counters?.directBytes?.addAndGet(n.toLong())
        bytesTransferred(n)
        return n
    }

    override fun getUri(): Uri? = uri

    override fun close() {
        source?.close()
        source = null
        uri = null
        chunk = ByteArray(0)
        chunkLength = 0
        if (opened) {
            opened = false
            transferEnded()
        }
    }
}
