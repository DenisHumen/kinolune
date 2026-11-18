// SPDX-License-Identifier: MPL-2.0
//
// Single import point for playback (R01-PLAYER `pa_player`, media_kit).
// `show` keeps pa_player's own Artwork/PaTokens from clashing with the app's.

export 'package:pa_player/pa_player.dart'
    show
        PlayRequest,
        PlayerScreen,
        MusicPlayerController,
        // MUS-018 audiobooks
        AudiobookRequest,
        BookPart,
        BookPlayerChapter,
        BookPlayerController,
        SleepTimer,
        NowPlayingBar,
        NowPlayingScreen,
        // MUS-003 queue and playlists
        QueueSource,
        SaveQueueCallback,
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
        // PLAY-004 track policy and sound processing
        AudioChoice,
        AudioProcessing,
        DownmixMode,
        LanguageMode,
        LanguagePreference,
        SdhPreference,
        SubtitleMode,
        TrackMemory,
        TrackPolicy,
        normalizeLanguage,
        SubtitleOverlay,
        SubtitleSettings,
        SystemCaptionStyle,
        // PLAY-006 subtitles drawn by libmpv (ASS styles, PGS, VobSub)
        SubtitleRenderOptions,
        // LIB-020 split movies as one timeline
        partsUri,
        // PLAY-009 seek previews
        TrickplayFrame,
        TrickplaySource,
        // PLAY-011 skip intro and credits
        SkipKind,
        SkipMode,
        SkipSegment,
        // UI-022 the macOS Playback menu
        MediaSessionCommand,
        SystemMediaSession;
