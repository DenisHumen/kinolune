# SPDX-License-Identifier: MPL-2.0
#
# Pinned sources of the Media3 FFmpeg audio extension (PLAY-AND-004).
# Media3 must be the same version as the app's `androidx.media3:*` dependencies
# (clients/android/player, pa_player_android): the JNI functions are registered
# against these exact Java classes. Bumping it means updating the version, the
# commit and every hash together in one commit.
#
# Only the files we build are fetched (the full androidx/media tarball is
# ~480 MB): raw files at the release commit, each checked against its SHA-256.
# The video renderer (ExperimentalFfmpegVideoRenderer) is left out on purpose:
# Media3 decodes video with MediaCodec, the extension is for audio only.

MEDIA3_VERSION=1.11.1
# Tag 1.11.1 (annotated) points at this commit.
MEDIA3_COMMIT=8c6678b657ede1e7883fc164ef73ed483c7796c3
MEDIA3_RAW="https://raw.githubusercontent.com/androidx/media/${MEDIA3_COMMIT}"
MEDIA3_LICENSE="Apache-2.0"

# <sha256> <path in androidx/media>
MEDIA3_FILES="
cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30 LICENSE
49b9ae90e4409e8cf12ee555ef308c92cb56516bb767e13b34cd051e3c9cd982 libraries/decoder_ffmpeg/proguard-rules.txt
68e6919f91cf810a86ee44ca8d045e60df2e45b31a75ed2985b8100043e4ad4b libraries/decoder_ffmpeg/src/main/jni/ffmpeg_jni.cc
64b5ff45fd9f60b6775d542bda141698e5d32cacdd04ddd96141e86f2c0a51f8 libraries/decoder_ffmpeg/src/main/java/androidx/media3/decoder/ffmpeg/FfmpegAudioDecoder.java
230bcaef26ddd233f7f2ac0cdd7a5896d98e7d8dd8d4c9b2ea1942c483b05c48 libraries/decoder_ffmpeg/src/main/java/androidx/media3/decoder/ffmpeg/FfmpegAudioRenderer.java
9e73968d0802b2d7f8add911ec4dc5a6e621777e997eca74332e173c5463ad7a libraries/decoder_ffmpeg/src/main/java/androidx/media3/decoder/ffmpeg/FfmpegDecoderException.java
a0843232be53210aefe97d8bc0a0268ac781cc7ce72e2d47bf42cd07f8048880 libraries/decoder_ffmpeg/src/main/java/androidx/media3/decoder/ffmpeg/FfmpegLibrary.java
1d75d02151fcec7a1c0036ee4a216e885ae91e1f85e8c9ddfbd72b4def6709be libraries/decoder_ffmpeg/src/main/java/androidx/media3/decoder/ffmpeg/package-info.java
"

# NDK of the FFmpeg/libmpv Android builds (libmpv.yml): one toolchain for
# the whole native set, and the same bytes from the same inputs.
ANDROID_NDK_VERSION=28.2.13676358 # r28c
ANDROID_API=24

# Decoders the extension is for. FFmpeg's own configure enables every decoder
# (tools/ffmpeg disables only encoders and devices); check-license.sh fails if
# one of these is missing from libavcodec.so. Matched by the codec's long name.
REQUIRED_DECODERS="truehd:TrueHD
mlp:MLP (Meridian Lossless Packing)
dca:DCA (DTS Coherent Acoustics)
eac3:ATSC A/52B (AC-3, E-AC-3)
ac3:ATSC A/52A (AC-3)"
