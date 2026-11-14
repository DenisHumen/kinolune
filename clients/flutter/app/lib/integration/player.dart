// SPDX-License-Identifier: MPL-2.0
//
// Single import point for playback (R01-PLAYER `pa_player`, media_kit).
// `show` keeps pa_player's own Artwork/PaTokens from clashing with the app's.

export 'package:pa_player/pa_player.dart'
    show
        PlayRequest,
        PlayerScreen,
        MusicPlayerController,
        NowPlayingBar,
        NowPlayingScreen,
        PaPlayerStrings,
        PlayerEngine,
        PlayerEngineFactory,
        UpNextItem,
        ensurePaPlayerInitialized,
        // PLAY-007 subtitles
        ExternalSubtitle,
        SubtitleAppearance,
        SubtitleBackground,
        SubtitleFont,
        SubtitleMemory,
        SubtitleOverlay,
        SubtitleSettings,
        SystemCaptionStyle,
        // PLAY-006 subtitles drawn by libmpv (ASS styles, PGS, VobSub)
        SubtitleRenderOptions,
        // LIB-020 split movies as one timeline
        partsUri,
        // PLAY-009 seek previews
        TrickplayFrame,
        TrickplaySource;
