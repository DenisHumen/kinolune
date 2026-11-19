// SPDX-License-Identifier: MPL-2.0
//
// Settings → Profiles (UI-015): the list of profiles and the editor with
// the parental controls of the core (LIB-012: rating limit and country,
// unrated video, artwork, folder white list) and of the app (the Streams
// section). Opened only with the parent PIN (profile_flows.ensureParent).

import 'dart:async';

import 'package:material_ui/material_ui.dart';

import '../integration/library.dart';
import '../l10n/l10n.dart';
import '../services/app_services.dart';
import '../sources/source_browser.dart';
import '../sources/source_manager.dart';
import '../theme/kinolune_theme.dart';
import '../widgets/settings_tiles.dart';
import 'pin_pad.dart';
import 'profile_avatar.dart';
import 'profile_controller.dart';
import 'profile_flows.dart';

/// Rating limits offered, as minimum viewer ages; null = no limit.
const kProfileAgeLimits = <int?>[null, 0, 6, 12, 16, 18];

/// Countries whose rating can win (ADR-0055 `certification_age`); '' = the
/// strictest known rating.
const kProfileRatingCountries = <String>[
  '',
  'US',
  'GB',
  'DE',
  'FR',
  'NL',
  'RU',
  'AU',
  'CA',
  'BR',
  'JP',
  'KR',
];

String _ageLabel(AppLocalizations l, int? age) =>
    age == null ? l.profilesRatingNone : l.profilesRatingUpTo(age);

/// "Kid · PIN" under a profile's name.
String profileSummary(
  AppLocalizations l,
  LibraryProfile p, {
  bool active = false,
  bool justWatch = false,
}) => [
  if (active) l.profilesActive,
  if (justWatch) l.profilesJustWatch,
  if (p.isKid) l.profilesKid,
  if (!p.isKid && ProfileController.isRestricted(p)) l.profilesRestricted,
  if (p.maxAge != null) _ageLabel(l, p.maxAge),
  if (p.hasPin) l.profilesHasPin,
].join(' · ');

class ProfilesScreen extends StatelessWidget {
  const ProfilesScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final ctl = Services.of(context).profileControl;
    return Scaffold(
      key: const ValueKey('profiles-screen'),
      appBar: AppBar(title: Text(l.profilesTitle)),
      body: ListenableBuilder(
        listenable: ctl,
        builder: (context, _) => ListView(
          padding: EdgeInsets.fromLTRB(
            d.pageInsets.left,
            8,
            d.pageInsets.right,
            d.pageInsets.bottom + 24,
          ),
          children: [
            for (final p in ctl.all)
              ListTile(
                key: ValueKey('profiles-item-${p.id}'),
                leading: ProfileAvatar.of(p),
                title: Text(profileName(l, p)),
                subtitle: Text(
                  profileSummary(
                    l,
                    p,
                    active: p.id == ctl.active?.id,
                    justWatch: ctl.justWatch(p),
                  ),
                ),
                trailing: const Icon(Icons.chevron_right),
                onTap: () => _open(context, ProfileEditorScreen(profile: p)),
              ),
            const SizedBox(height: 16),
            Wrap(
              spacing: 12,
              runSpacing: 12,
              children: [
                OutlinedButton.icon(
                  key: const ValueKey('profiles-add'),
                  onPressed: () => _open(context, const ProfileEditorScreen()),
                  icon: const Icon(Icons.person_add_alt_1_outlined),
                  label: Text(l.profilesAdd),
                ),
                OutlinedButton.icon(
                  key: const ValueKey('profiles-add-kid'),
                  onPressed: () =>
                      _open(context, const ProfileEditorScreen(kid: true)),
                  icon: const Icon(Icons.child_care_outlined),
                  label: Text(l.profilesAddKid),
                ),
              ],
            ),
            const SizedBox(height: 16),
            Text(l.profilesNote, style: Theme.of(context).textTheme.bodySmall),
          ],
        ),
      ),
    );
  }

  static void _open(BuildContext context, Widget screen) => unawaited(
    Navigator.of(context).push(MaterialPageRoute<void>(builder: (_) => screen)),
  );
}

/// Creates ([profile] null) or edits a profile.
class ProfileEditorScreen extends StatefulWidget {
  const ProfileEditorScreen({super.key, this.profile, this.kid = false});

  final LibraryProfile? profile;

  /// A new profile starts as a kid profile with the safe defaults.
  final bool kid;

  @override
  State<ProfileEditorScreen> createState() => _ProfileEditorScreenState();
}

class _ProfileEditorScreenState extends State<ProfileEditorScreen> {
  late final _name = TextEditingController(text: widget.profile?.name ?? '');
  late ProfileSettings _settings =
      widget.profile?.settings ??
      (widget.kid
          ? const ProfileSettings.kid(name: '')
          : const ProfileSettings(name: ''));
  late AvatarRef _avatar = AvatarRef.parse(
    widget.profile?.avatar ?? (widget.kid ? 'icon:0:3' : 'color:1'),
  );
  List<ProfileFolder> _folders = const [];
  bool? _hideTwitch;
  bool _justWatch = false;
  String? _newPin;
  bool _saving = false;
  bool _foldersLoaded = false;

  ProfileController get _ctl => Services.read(context).profileControl;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_foldersLoaded) return;
    _foldersLoaded = true;
    final p = widget.profile;
    if (p != null) {
      _hideTwitch = _ctl.hidesTwitch(p);
      _justWatch = _ctl.justWatch(p);
      unawaited(
        _ctl.folders(p.id).then((f) {
          if (mounted) setState(() => _folders = f);
        }),
      );
    }
  }

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  bool get _restricted =>
      _settings.isKid ||
      _settings.maxAge != null ||
      _settings.hideUnrated ||
      _settings.hideArtwork ||
      _settings.restrictFolders;

  /// Hidden by default for restricted profiles (owner decision).
  bool get _effectiveHideTwitch => _hideTwitch ?? _restricted;

  void _set(ProfileSettings s) => setState(() => _settings = s);

  void _setKid(bool on) => _set(
    on
        ? ProfileSettings.kid(
            name: _settings.name,
            avatar: _settings.avatar,
            maxAge: _settings.maxAge ?? 12,
            ratingCountry: _settings.ratingCountry,
          )
        : _settings.copyWith(isKid: false),
  );

  Future<void> _setPin() async {
    final l = context.l10n;
    final p = widget.profile;
    final pin = await showNewPinDialog(context, title: l.profilesPinSetTitle);
    if (pin == null || !mounted) return;
    if (p == null) {
      setState(() => _newPin = pin);
      return;
    }
    try {
      await _ctl.setPin(p.id, pin);
      if (mounted) setState(() {});
    } catch (e) {
      if (mounted) sayProfileError(context, e);
    }
  }

  Future<void> _removePin() async {
    final p = widget.profile;
    if (p == null) {
      setState(() => _newPin = null);
      return;
    }
    try {
      await _ctl.setPin(p.id, null);
      if (mounted) setState(() {});
    } catch (e) {
      if (mounted) sayProfileError(context, e);
    }
  }

  bool get _hasPin {
    final p = widget.profile;
    if (p == null) return _newPin != null;
    return _ctl.all.where((e) => e.id == p.id).firstOrNull?.hasPin ?? p.hasPin;
  }

  Future<void> _addFolder() async {
    final f = await showDialog<ProfileFolder>(
      context: context,
      builder: (_) => const _FolderPickerDialog(),
    );
    if (f == null || !mounted || _folders.contains(f)) return;
    setState(() => _folders = [..._folders, f]);
  }

  Future<void> _save() async {
    final l = context.l10n;
    final name = _name.text.trim();
    if (name.isEmpty) {
      ScaffoldMessenger.maybeOf(context)
          ?.showSnackBar(SnackBar(content: Text(l.profilesNameRequired)));
      return;
    }
    final s = _settings.copyWith(name: name, avatar: _avatar.ref);
    final id = widget.profile?.id;
    if (_ctl.needsParentPinFor(s, id: id) &&
        !await ensureParentPinExists(context)) {
      return;
    }
    if (!mounted) return;
    // UI-047: the mode is left with a parent PIN of another profile.
    if (_justWatch && _ctl.needsParentPinForJustWatch(id: id)) {
      final mayAsk = _ctl.active != null && _ctl.active!.id != id;
      if (mayAsk && !_ctl.hasParentPin) await ensureParentPinExists(context);
      if (!mounted) return;
      if (_ctl.needsParentPinForJustWatch(id: id)) {
        sayProfileError(
          context,
          const ProfileChangeException(ProfileChangeError.needsParentPin),
        );
        return;
      }
    }
    setState(() => _saving = true);
    try {
      if (id == null) {
        await _ctl.create(
          s,
          pin: _justWatch ? null : _newPin,
          folders: _folders,
          hideTwitch: _hideTwitch,
          justWatch: _justWatch,
        );
      } else {
        await _ctl.update(
          id,
          s,
          folders: _folders,
          hideTwitch: _hideTwitch,
          justWatch: _justWatch,
        );
      }
      if (mounted) Navigator.of(context).pop();
    } catch (e) {
      if (mounted) sayProfileError(context, e);
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  Future<void> _delete() async {
    final l = context.l10n;
    final p = widget.profile!;
    final ok = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        title: Text(l.profilesDeleteTitle(profileName(l, p))),
        content: Text(l.profilesDeleteBody),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(c, false),
            child: Text(l.commonCancel),
          ),
          FilledButton(
            key: const ValueKey('profile-delete-confirm'),
            onPressed: () => Navigator.pop(c, true),
            child: Text(l.profilesDelete),
          ),
        ],
      ),
    );
    if (ok != true || !mounted) return;
    try {
      await _ctl.delete(p.id);
      if (mounted) Navigator.of(context).pop();
    } catch (e) {
      if (mounted) sayProfileError(context, e);
    }
  }

  String _folderLabel(AppLocalizations l, ProfileFolder f) {
    final services = Services.read(context);
    final source =
        services.managedSource(f.sourceId)?.name ??
        services.sources.where((s) => s.id == f.sourceId).firstOrNull?.name ??
        f.sourceId;
    return f.relPath.isEmpty
        ? l.profilesFolderWholeSource(source)
        : '$source / ${f.relPath}';
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final d = KDensity.of(context);
    final theme = Theme.of(context);
    final p = widget.profile;
    final ctl = Services.of(context).profileControl;
    final isActive = p != null && p.id == ctl.active?.id;
    final canDelete = p != null && !isActive && ctl.all.length > 1;
    return Scaffold(
      key: const ValueKey('profile-editor'),
      appBar: AppBar(
        title: Text(
          p == null
              ? (widget.kid ? l.profilesAddKid : l.profilesAdd)
              : l.profilesEdit,
        ),
      ),
      body: ListView(
        padding: EdgeInsets.fromLTRB(
          d.pageInsets.left,
          8,
          d.pageInsets.right,
          d.pageInsets.bottom + 32,
        ),
        children: [
          Align(
            alignment: Alignment.topLeft,
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 760),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      ValueListenableBuilder(
                        valueListenable: _name,
                        builder: (context, v, _) => ProfileAvatar(
                          avatar: _avatar,
                          name: v.text,
                          size: 56,
                          kid: _settings.isKid,
                        ),
                      ),
                      const SizedBox(width: 16),
                      Expanded(
                        child: TextField(
                          key: const ValueKey('profile-name'),
                          controller: _name,
                          maxLength: 40,
                          decoration: InputDecoration(
                            labelText: l.profilesName,
                          ),
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 12),
                  ValueListenableBuilder(
                    valueListenable: _name,
                    builder: (context, v, _) => AvatarPicker(
                      value: _avatar,
                      name: v.text,
                      onChanged: (a) => setState(() => _avatar = a),
                    ),
                  ),
                  SettingsSection(
                    key: const ValueKey('profile-section-just-watch'),
                    icon: Icons.smart_display_outlined,
                    title: l.profilesJustWatch,
                    subtitle: l.profilesJustWatchSectionSubtitle,
                    children: [
                      SettingsSwitchTile(
                        key: const ValueKey('profile-just-watch'),
                        icon: Icons.smart_display_outlined,
                        title: l.profilesJustWatch,
                        subtitle: l.profilesJustWatchSubtitle,
                        value: _justWatch,
                        onChanged: (v) => setState(() => _justWatch = v),
                      ),
                      if (_justWatch)
                        Padding(
                          key: const ValueKey('profile-just-watch-no-pin'),
                          padding: const EdgeInsets.only(top: 8),
                          child: Text(
                            l.profilesJustWatchNoPin,
                            style: theme.textTheme.bodySmall,
                          ),
                        ),
                    ],
                  ),
                  // A "just watch" profile has no PIN of its own (UI-047).
                  if (!_justWatch)
                  SettingsSection(
                    key: const ValueKey('profile-section-pin'),
                    icon: Icons.pin_outlined,
                    title: l.profilesPinSection,
                    subtitle: l.profilesPinSectionSubtitle,
                    children: [
                      SettingsTile(
                        key: const ValueKey('profile-pin'),
                        icon: Icons.password,
                        title: _hasPin ? l.profilesPinChange : l.profilesPinSet,
                        onTap: _setPin,
                      ),
                      if (_hasPin)
                        SettingsTile(
                          key: const ValueKey('profile-pin-remove'),
                          icon: Icons.lock_open_outlined,
                          title: l.profilesPinRemove,
                          onTap: _removePin,
                          trailing: const SizedBox.shrink(),
                        ),
                    ],
                  ),
                  SettingsSection(
                    key: const ValueKey('profile-section-parental'),
                    icon: Icons.family_restroom,
                    title: l.profilesParentalTitle,
                    subtitle: l.profilesParentalSubtitle,
                    children: [
                      SettingsSwitchTile(
                        key: const ValueKey('profile-kid'),
                        icon: Icons.child_care_outlined,
                        title: l.profilesKidProfile,
                        subtitle: l.profilesKidProfileSubtitle,
                        value: _settings.isKid,
                        onChanged: _setKid,
                      ),
                      SettingsChoiceTile<int?>(
                        key: const ValueKey('profile-max-age'),
                        icon: Icons.eighteen_up_rating_outlined,
                        title: l.profilesRatingLimit,
                        hint: l.profilesRatingLimitSubtitle,
                        value: _settings.maxAge,
                        options: kProfileAgeLimits,
                        label: (a) => _ageLabel(l, a),
                        onChanged: (a) =>
                            _set(_settings.copyWith(maxAge: () => a)),
                      ),
                      SettingsChoiceTile<String>(
                        key: const ValueKey('profile-country'),
                        icon: Icons.public,
                        title: l.profilesRatingCountry,
                        hint: l.profilesRatingCountrySubtitle,
                        value: _settings.ratingCountry,
                        options: kProfileRatingCountries,
                        label: (c) =>
                            c.isEmpty ? l.profilesRatingCountryStrictest : c,
                        onChanged: (c) =>
                            _set(_settings.copyWith(ratingCountry: c)),
                      ),
                      SettingsSwitchTile(
                        key: const ValueKey('profile-hide-unrated'),
                        icon: Icons.help_outline,
                        title: l.profilesHideUnrated,
                        subtitle: l.profilesHideUnratedSubtitle,
                        value: _settings.hideUnrated,
                        onChanged: (v) =>
                            _set(_settings.copyWith(hideUnrated: v)),
                      ),
                      SettingsSwitchTile(
                        key: const ValueKey('profile-hide-artwork'),
                        icon: Icons.hide_image_outlined,
                        title: l.profilesHideArtwork,
                        subtitle: l.profilesHideArtworkSubtitle,
                        value: _settings.hideArtwork,
                        onChanged: (v) =>
                            _set(_settings.copyWith(hideArtwork: v)),
                      ),
                      SettingsSwitchTile(
                        key: const ValueKey('profile-hide-twitch'),
                        icon: Icons.live_tv_outlined,
                        title: l.profilesHideTwitch,
                        subtitle: l.profilesHideTwitchSubtitle,
                        value: _effectiveHideTwitch,
                        onChanged: (v) => setState(() => _hideTwitch = v),
                      ),
                      SettingsSwitchTile(
                        key: const ValueKey('profile-restrict-folders'),
                        icon: Icons.folder_special_outlined,
                        title: l.profilesRestrictFolders,
                        subtitle: l.profilesRestrictFoldersSubtitle,
                        value: _settings.restrictFolders,
                        onChanged: (v) =>
                            _set(_settings.copyWith(restrictFolders: v)),
                      ),
                      if (_settings.restrictFolders) ...[
                        if (_folders.isEmpty)
                          Padding(
                            padding: const EdgeInsets.symmetric(vertical: 8),
                            child: Text(
                              l.profilesFoldersEmpty,
                              style: theme.textTheme.bodySmall,
                            ),
                          ),
                        for (final f in _folders)
                          SettingsTile(
                            key: ValueKey(
                              'profile-folder-${f.sourceId}-${f.relPath}',
                            ),
                            icon: Icons.folder_outlined,
                            title: _folderLabel(l, f),
                            trailing: IconButton(
                              tooltip: l.profilesFolderRemove,
                              onPressed: () => setState(
                                () => _folders = [
                                  for (final e in _folders)
                                    if (e != f) e,
                                ],
                              ),
                              icon: const Icon(Icons.close),
                            ),
                          ),
                        Align(
                          alignment: Alignment.centerLeft,
                          child: TextButton.icon(
                            key: const ValueKey('profile-add-folder'),
                            onPressed: _addFolder,
                            icon: const Icon(Icons.create_new_folder_outlined),
                            label: Text(l.profilesFolderAdd),
                          ),
                        ),
                        if (ctl.restricted)
                          Text(
                            l.profilesFoldersFromKid,
                            style: theme.textTheme.bodySmall,
                          ),
                      ],
                    ],
                  ),
                  const SizedBox(height: 24),
                  Wrap(
                    spacing: 12,
                    runSpacing: 12,
                    children: [
                      FilledButton(
                        key: const ValueKey('profile-save'),
                        onPressed: _saving ? null : _save,
                        child: Text(l.commonSave),
                      ),
                      if (canDelete)
                        OutlinedButton(
                          key: const ValueKey('profile-delete'),
                          onPressed: _delete,
                          child: Text(l.profilesDelete),
                        ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// Picks a source or a folder in it for the white list.
class _FolderPickerDialog extends StatefulWidget {
  const _FolderPickerDialog();

  @override
  State<_FolderPickerDialog> createState() => _FolderPickerDialogState();
}

class _FolderPickerDialogState extends State<_FolderPickerDialog> {
  ManagedSource? _source;
  String _dir = '';
  List<BrowseEntry>? _entries;
  Object? _error;
  late final SourceBrowser _browser;
  bool _init = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_init) return;
    _init = true;
    final s = Services.read(context);
    _browser = SourceBrowser.of(s.library, s.sourceManager);
  }

  Future<void> _open(ManagedSource source, String dir) async {
    setState(() {
      _source = source;
      _dir = dir;
      _entries = null;
      _error = null;
    });
    try {
      final listing = await _browser.list(source, dir);
      if (!mounted) return;
      final folders = [
        for (final e in listing.entries)
          if (e.isFolder && !isHiddenName(e.name)) e,
      ]..sort((a, b) => a.name.toLowerCase().compareTo(b.name.toLowerCase()));
      setState(() => _entries = folders);
    } catch (e) {
      if (mounted) setState(() => _error = e);
    }
  }

  void _up() {
    final source = _source;
    if (source == null) return;
    if (_dir.isEmpty) {
      setState(() => _source = null);
      return;
    }
    final i = _dir.lastIndexOf('/');
    unawaited(_open(source, i < 0 ? '' : _dir.substring(0, i)));
  }

  @override
  Widget build(BuildContext context) {
    final l = context.l10n;
    final services = Services.of(context);
    final source = _source;
    final List<Widget> rows;
    if (source == null) {
      rows = [
        if (services.managedSources.isEmpty)
          Padding(
            padding: const EdgeInsets.all(16),
            child: Text(l.settingsNoSources),
          ),
        for (final s in services.managedSources)
          ListTile(
            key: ValueKey('profile-folder-source-${s.id}'),
            leading: const Icon(Icons.storage_outlined),
            title: Text(s.name),
            trailing: const Icon(Icons.chevron_right),
            onTap: () => _open(s, ''),
          ),
      ];
    } else if (_error != null) {
      rows = [
        Padding(
          padding: const EdgeInsets.all(16),
          child: Text(l.profilesFolderListFailed),
        ),
      ];
    } else if (_entries == null) {
      rows = const [
        Padding(
          padding: EdgeInsets.all(24),
          child: Center(child: CircularProgressIndicator()),
        ),
      ];
    } else {
      rows = [
        for (final e in _entries!)
          ListTile(
            key: ValueKey('profile-folder-entry-${e.relPath}'),
            leading: const Icon(Icons.folder_outlined),
            title: Text(e.name),
            trailing: const Icon(Icons.chevron_right),
            onTap: () => _open(source, e.relPath),
          ),
      ];
    }
    return AlertDialog(
      title: Text(
        source == null
            ? l.profilesFolderAdd
            : (_dir.isEmpty ? source.name : '${source.name} / $_dir'),
      ),
      content: SizedBox(
        width: 480,
        height: 360,
        child: ListView(children: rows),
      ),
      actions: [
        if (source != null)
          TextButton(
            key: const ValueKey('profile-folder-up'),
            onPressed: _up,
            child: Text(l.profilesFolderUp),
          ),
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(l.commonCancel),
        ),
        if (source != null)
          FilledButton(
            key: const ValueKey('profile-folder-choose'),
            onPressed: () => Navigator.pop(
              context,
              ProfileFolder(sourceId: source.id, relPath: _dir),
            ),
            child: Text(l.profilesFolderChoose),
          ),
      ],
    );
  }
}
