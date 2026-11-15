// SPDX-License-Identifier: MPL-2.0
//
// Pages of the music library (MUS-004, 01 §4.11): album (format badge,
// discs, more by the artist), artist (popular tracks, albums, appears on)
// and a track list (genre, composer, folder, mix, playlist). Albums and
// playlists of network tracks can be downloaded for offline (NET-019).

import 'package:material_ui/material_ui.dart';

import '../../integration/library.dart';
import '../../l10n/l10n.dart';
import '../../offline/offline_ui.dart';
import '../../services/app_services.dart';
import '../../theme/kinolune_theme.dart';
import '../../widgets/artwork.dart';
import '../../widgets/focusable.dart';
import '../../widgets/format.dart';
import 'music_play.dart';
import 'playlist_io.dart';
import 'music_widgets.dart';

void openAlbum(BuildContext context, MusicIndex index, MusicAlbum album) =>
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => AlbumScreen(index: index, album: album),
      ),
    );

void openArtist(BuildContext context, MusicIndex index, MusicArtist artist) =>
    Navigator.of(context).push(
      MaterialPageRoute<void>(
        builder: (_) => ArtistScreen(index: index, artist: artist),
      ),
    );

void openTrackList(
  BuildContext context, {
  required String overline,
  required String title,
  required List<MusicTrack> tracks,
  String? artUrl,
  String? artSeed,
}) => Navigator.of(context).push(
  MaterialPageRoute<void>(
    builder: (_) => TrackListScreen(
      overline: overline,
      title: title,
      tracks: tracks,
      artUrl: artUrl,
      artSeed: artSeed ?? title,
    ),
  ),
);

/// A track row that plays [list] from [i] ([source] names the list in the
/// queue); long press or right click opens the track menu: Play Next, Add
/// to Queue, Add to playlist (MUS-003).
Widget musicTrackRow(
  BuildContext context,
  List<MusicTrack> list,
  int i, {
  bool showArt = false,
  bool badge = false,
  int? number,
  String? subtitle,
  String? source,
}) {
  final t = list[i];
  final music = libraryMusic(Services.read(context).library);
  return TrackTile(
    key: ValueKey('track-${t.id}'),
    track: t.item,
    showArt: showArt,
    number: number,
    subtitle: subtitle,
    badge: badge ? t.formatLabel : null,
    onTap: () => playMusic(context, list, index: i, source: source),
    onContextMenu: (_) => showTrackMenu(context, music, t),
  );
}

/// The track menu: Play Next, Add to Queue and, when the library keeps
/// playlists, Add to playlist.
Future<void> showTrackMenu(
  BuildContext context,
  LibraryMusic music,
  MusicTrack track,
) async {
  final l = context.l10n;
  final choice = await showDialog<String>(
    context: context,
    builder: (context) => SimpleDialog(
      title: Text(track.title, maxLines: 2, overflow: TextOverflow.ellipsis),
      children: [
        SimpleDialogOption(
          key: const ValueKey('track-menu-next'),
          onPressed: () => Navigator.pop(context, 'next'),
          child: ListTile(
            leading: const Icon(Icons.playlist_play_rounded),
            title: Text(l.musicPlayNext),
          ),
        ),
        SimpleDialogOption(
          key: const ValueKey('track-menu-queue'),
          onPressed: () => Navigator.pop(context, 'queue'),
          child: ListTile(
            leading: const Icon(Icons.queue_music_rounded),
            title: Text(l.musicAddToQueue),
          ),
        ),
        if (music.hasMusicPlaylists)
          SimpleDialogOption(
            key: const ValueKey('track-menu-playlist'),
            onPressed: () => Navigator.pop(context, 'playlist'),
            child: ListTile(
              leading: const Icon(Icons.playlist_add_rounded),
              title: Text(l.musicAddToPlaylist),
            ),
          ),
      ],
    ),
  );
  if (!context.mounted) return;
  switch (choice) {
    case 'next':
      await queueMusic(context, [track], next: true);
    case 'queue':
      await queueMusic(context, [track], next: false);
    case 'playlist':
      await showAddToMusicPlaylist(context, music, track);
  }
}

/// Picks an audio playlist (or makes a new one) and adds [track] to it.
Future<void> showAddToMusicPlaylist(
  BuildContext context,
  LibraryMusic music,
  MusicTrack track,
) async {
  final playlists = await music.musicPlaylists().first;
  if (!context.mounted) return;
  final l = context.l10n;
  final name = TextEditingController();
  final chosen = await showDialog<Object>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(l.musicAddToPlaylist),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (playlists.isEmpty) Text(l.musicPlaylistsEmpty),
              for (final p in playlists)
                ListTile(
                  contentPadding: EdgeInsets.zero,
                  leading: const Icon(Icons.queue_music),
                  title: Text(p.name),
                  subtitle: Text(l.musicPlaylistTrackCount(p.trackCount)),
                  onTap: () => Navigator.pop(context, p),
                ),
              const SizedBox(height: 8),
              TextField(
                controller: name,
                decoration: InputDecoration(labelText: l.libraryPlaylistName),
                onSubmitted: (v) => Navigator.pop(context, v.trim()),
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.commonCancel),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context, name.text.trim()),
          child: Text(l.libraryPlaylistCreate),
        ),
      ],
    ),
  );
  name.dispose();
  if (!context.mounted) return;
  final messenger = ScaffoldMessenger.maybeOf(context);
  final MusicPlaylist playlist;
  switch (chosen) {
    case final MusicPlaylist p:
      playlist = p;
    case final String n when n.isNotEmpty:
      playlist = await music.createMusicPlaylist(n);
    default:
      return;
  }
  final added = await music.addToMusicPlaylist(playlist.id, track.id);
  if (!context.mounted) return;
  messenger
    ?..removeCurrentSnackBar()
    ..showSnackBar(
      SnackBar(
        content: Text(
          added
              ? l.libraryPlaylistAdded(playlist.name)
              : l.libraryPlaylistAlready(playlist.name),
        ),
      ),
    );
}

List<Widget> _playButtons(
  BuildContext context,
  List<MusicTrack> tracks, {
  bool autofocus = false,
  String? source,
}) {
  final l = context.l10n;
  return [
    KButton(
      label: l.musicPlay,
      icon: Icons.play_arrow_rounded,
      primary: true,
      autofocus: autofocus,
      onPressed: () => playMusic(context, tracks, source: source),
    ),
    KButton(
      label: l.commonShuffle,
      icon: Icons.shuffle,
      onPressed: () =>
          playMusic(context, tracks, shuffle: true, source: source),
    ),
  ];
}

class AlbumScreen extends StatelessWidget {
  const AlbumScreen({super.key, required this.index, required this.album});

  final MusicIndex index;
  final MusicAlbum album;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final tracks = album.tracks;
    final format = album.formatLabel;
    final artist = album.artist == null
        ? null
        : index.artistNamed(album.artist!);
    final more = [
      for (final a in artist?.albums ?? const <MusicAlbum>[])
        if (a.key != album.key) a,
    ];
    final discs = album.isMultiDisc ? album.discs : const [1];
    return MusicPage(
      slivers: [
        SliverPadding(
          padding: musicPagePadding(d, bottom: 24),
          sliver: SliverToBoxAdapter(
            child: MusicPageHeader(
              art: Artwork(
                url: album.artUrl,
                seed: album.artSeed,
                style: ArtStyle.album,
              ),
              overline: l.musicAlbumLabel,
              title: albumTitle(l, album),
              facts: [
                albumArtist(l, album),
                if (album.year != null) '${album.year}',
                l.musicTrackCount(tracks.length),
                formatRuntime(l, album.durationMs),
              ].where((s) => s.isNotEmpty).join('  ·  '),
              badges: [
                if (format != null) FormatBadge(format, hiRes: album.isHiRes),
              ],
              buttons: [
                ..._playButtons(
                  context,
                  tracks,
                  autofocus: !d.isPhone,
                  source: albumTitle(l, album),
                ),
                if (artist != null && !d.isTv)
                  KButton(
                    label: artist.name,
                    icon: Icons.person_outline,
                    onPressed: () => openArtist(context, index, artist),
                  ),
                // NET-019: the album's network tracks on the device.
                ?offlineListButton(
                  context,
                  key: const ValueKey('music-album-download'),
                  label: l.offlineDownloadAlbum,
                  items: [for (final t in tracks) t.item],
                ),
              ],
            ),
          ),
        ),
        for (final disc in discs) ...[
          if (discs.length > 1)
            SliverPadding(
              padding: musicPagePadding(d),
              sliver: SliverToBoxAdapter(
                child: Padding(
                  key: ValueKey('disc-$disc'),
                  padding: const EdgeInsets.fromLTRB(10, 16, 10, 6),
                  child: Row(
                    children: [
                      const Icon(
                        Icons.album_outlined,
                        size: 18,
                        color: KColors.accent,
                      ),
                      const SizedBox(width: 8),
                      Text(
                        l.musicDisc(disc),
                        style: Theme.of(context).textTheme.titleSmall,
                      ),
                    ],
                  ),
                ),
              ),
            ),
          SliverPadding(
            padding: musicPagePadding(d),
            sliver: SliverList.list(
              children: [
                for (var i = 0; i < tracks.length; i++)
                  if ((tracks[i].discNumber ?? 1) == disc || discs.length == 1)
                    musicTrackRow(
                      context,
                      tracks,
                      i,
                      source: albumTitle(l, album),
                      subtitle:
                          tracks[i].artists.length > 1 ||
                              tracks[i].artist != album.artist
                          ? tracks[i].artists.join(', ')
                          : null,
                    ),
              ],
            ),
          ),
        ],
        if (more.isNotEmpty)
          SliverToBoxAdapter(
            child: Padding(
              padding: const EdgeInsets.only(top: 28),
              child: AlbumRow(
                title: l.musicMoreBy(artist!.name),
                albums: more,
                onOpen: (a) => openAlbum(context, index, a),
              ),
            ),
          ),
      ],
    );
  }
}

class ArtistScreen extends StatelessWidget {
  const ArtistScreen({super.key, required this.index, required this.artist});

  final MusicIndex index;
  final MusicArtist artist;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final popular = artist.popular.take(5).toList();
    return MusicPage(
      slivers: [
        SliverPadding(
          padding: musicPagePadding(d, bottom: 8),
          sliver: SliverToBoxAdapter(
            child: MusicPageHeader(
              art: ClipOval(
                child: Artwork(
                  url: artist.artUrl,
                  seed: artist.artSeed,
                  style: ArtStyle.album,
                ),
              ),
              overline: l.musicArtistLabel,
              title: artist.name,
              facts: [
                l.musicAlbumCount(artist.albums.length),
                l.musicTrackCount(artist.tracks.length),
              ].join('  ·  '),
              buttons: _playButtons(
                context,
                artist.tracks,
                autofocus: !d.isPhone,
                source: artist.name,
              ),
            ),
          ),
        ),
        if (popular.isNotEmpty)
          SliverPadding(
            padding: musicPagePadding(d),
            sliver: SliverList.list(
              children: [
                MusicSectionTitle(l.musicPopular),
                for (var i = 0; i < popular.length; i++)
                  musicTrackRow(
                    context,
                    popular,
                    i,
                    source: artist.name,
                    showArt: true,
                    subtitle: popular[i].playCount > 0
                        ? l.musicPlayCount(popular[i].playCount)
                        : popular[i].item.album,
                  ),
              ],
            ),
          ),
        if (artist.albums.isNotEmpty)
          SliverToBoxAdapter(
            child: Padding(
              padding: const EdgeInsets.only(top: 24),
              child: AlbumRow(
                title: l.musicSectionAlbums,
                albums: artist.albums,
                onOpen: (a) => openAlbum(context, index, a),
              ),
            ),
          ),
        if (artist.appearsOn.isNotEmpty)
          SliverToBoxAdapter(
            child: Padding(
              padding: const EdgeInsets.only(top: 24),
              child: AlbumRow(
                title: l.musicAppearsOn,
                albums: artist.appearsOn,
                onOpen: (a) => openAlbum(context, index, a),
              ),
            ),
          ),
      ],
    );
  }
}

/// Tracks of a genre, a composer, a folder, a mix or a playlist.
class TrackListScreen extends StatelessWidget {
  const TrackListScreen({
    super.key,
    required this.overline,
    required this.title,
    required this.tracks,
    required this.artSeed,
    this.artUrl,
    this.actions = const [],
  });

  final String overline;
  final String title;
  final List<MusicTrack> tracks;
  final String? artUrl;
  final String artSeed;

  /// More header buttons after Play and Shuffle (a playlist's export…).
  final List<Widget> actions;

  @override
  Widget build(BuildContext context) {
    final d = KDensity.of(context);
    final l = context.l10n;
    final total = tracks.fold<int>(0, (s, t) => s + (t.item.durationMs ?? 0));
    return MusicPage(
      slivers: [
        SliverPadding(
          padding: musicPagePadding(d, bottom: 20),
          sliver: SliverToBoxAdapter(
            child: MusicPageHeader(
              art: Artwork(url: artUrl, seed: artSeed, style: ArtStyle.album),
              overline: overline,
              title: title,
              facts: [
                l.musicTrackCount(tracks.length),
                formatRuntime(l, total),
              ].where((s) => s.isNotEmpty).join('  ·  '),
              buttons: [
                ..._playButtons(
                  context,
                  tracks,
                  autofocus: !d.isPhone,
                  source: title,
                ),
                ...actions,
              ],
            ),
          ),
        ),
        SliverPadding(
          padding: musicPagePadding(d),
          sliver: SliverList.builder(
            itemCount: tracks.length,
            itemBuilder: (context, i) => musicTrackRow(
              context,
              tracks,
              i,
              source: title,
              showArt: true,
              badge: !d.isPhone,
            ),
          ),
        ),
      ],
    );
  }
}

/// A playlist page: its tracks are loaded from the library.
class MusicPlaylistScreen extends StatelessWidget {
  const MusicPlaylistScreen({
    super.key,
    required this.index,
    required this.playlist,
  });

  final MusicIndex index;
  final MusicPlaylist playlist;

  @override
  Widget build(BuildContext context) {
    final music = libraryMusic(Services.read(context).library);
    final byId = {for (final t in index.tracks) t.id: t};
    return FutureBuilder<List<LibraryItem>>(
      future: music.musicPlaylistItems(playlist.id),
      builder: (context, snap) {
        final tracks = [
          for (final i in snap.data ?? const <LibraryItem>[])
            byId[i.id] ?? MusicTrack.fromItem(i),
        ];
        return TrackListScreen(
          overline: context.l10n.musicPlaylistLabel,
          title: playlist.name,
          tracks: tracks,
          artUrl: tracks.isEmpty ? null : tracks.first.item.posterUrl,
          artSeed: tracks.isEmpty ? playlist.name : tracks.first.folder,
          actions: [
            ?offlineListButton(
              context,
              key: const ValueKey('music-playlist-download'),
              label: context.l10n.offlineDownloadPlaylist,
              items: [for (final t in tracks) t.item],
            ),
            KButton(
              key: const ValueKey('music-playlist-export'),
              label: context.l10n.musicM3uExport,
              icon: Icons.file_upload_outlined,
              onPressed: tracks.isEmpty
                  ? null
                  : () => exportM3u(context, playlist.name, tracks),
            ),
          ],
        );
      },
    );
  }
}
