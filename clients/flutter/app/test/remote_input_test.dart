// SPDX-License-Identifier: MPL-2.0
//
// UI-027: typing on the TV from a phone in the app. A text field focused on
// the TV asks the paired phone (two FakeDeviceSync on one fake LAN); what
// the phone sends lands in the field and the TV shows who types; leaving
// the field cancels. On the phone the input sheet opens by itself (or a
// notification while in the background) and closes when the TV cancels.
// The core path between two real sync services is in pa_core
// test/sync_test.dart and pa-sync tests/service.rs.
//
// With KINOLUNE_SCREENSHOTS_DIR set the frames go to
// docs/screenshots/progress/ui-027:
//   KINOLUNE_SCREENSHOTS_DIR=$PWD/../../../docs/screenshots/progress/ui-027 \
//     flutter test test/remote_input_test.dart

import 'package:flutter_test/flutter_test.dart';
import 'package:kinolune/fakes/fake_device_sync.dart';
import 'package:kinolune/l10n/l10n.dart';
import 'package:kinolune/notices/in_app_notices.dart';
import 'package:kinolune/notices/notice_host.dart';
import 'package:kinolune/platform/form_factor.dart';
import 'package:kinolune/shell/app_shell.dart';
import 'package:kinolune/sync/device_sync.dart';
import 'package:kinolune/sync/remote_input.dart';
import 'package:material_ui/material_ui.dart';

import 'support/test_app.dart';

final ru = lookupAppLocalizations(const Locale('ru'));

/// A TV and a phone paired on one fake LAN.
Future<(FakeDeviceSync, FakeDeviceSync)> _pair() async {
  final lan = FakeSyncLan();
  final tv = FakeDeviceSync(
    id: 'ktv',
    name: 'Living room TV', // l10n-ignore
    kind: SyncDeviceKind.tv,
    lan: lan,
  );
  final phone = FakeDeviceSync(
    id: 'kphone',
    name: 'Pixel 8',
    kind: SyncDeviceKind.phone,
    lan: lan,
  );
  await phone.join((await tv.invite()).code);
  return (tv, phone);
}

/// Input frames a fake receives.
List<RemoteInputFrame> _frames(WidgetTester t, FakeDeviceSync d) {
  final out = <RemoteInputFrame>[];
  final sub = d.events.listen((e) {
    if (e.input case final i?) out.add(i);
  });
  addTearDown(sub.cancel);
  return out;
}

class _FakeNotifier implements RemoteInputNotifier {
  final shown = <String>[];
  int cancels = 0;
  int asks = 0;

  @override
  Future<void> ensurePermission() async => asks++;

  @override
  Future<void> show({required String title, required String body}) async =>
      shown.add('$title | $body');

  @override
  Future<void> cancel() async => cancels++;
}

/// A bare app around [home] with remote input on.
Widget _harness({
  required DeviceSync sync,
  required InAppNotices notices,
  required bool tv,
  RemoteInputNotifier? notifier,
  required Widget home,
}) {
  final key = GlobalKey<NavigatorState>();
  return MaterialApp(
    navigatorKey: key,
    locale: const Locale('ru'),
    supportedLocales: AppLocalizations.supportedLocales,
    localizationsDelegates: kLocalizationsDelegates,
    builder: (context, child) => RemoteInputHost(
      sync: sync,
      notices: notices,
      navigatorKey: key,
      tv: tv,
      phone: !tv,
      notifier: notifier,
      child: InAppNoticeHost(
        notices: notices,
        navigatorKey: key,
        child: child!,
      ),
    ),
    home: home,
  );
}

void main() {
  setUpAll(loadAppFonts);

  testWidgets('TV: the search field asks the phone, the text arrives, '
      'leaving the field cancels', (t) async {
    setSurface(t, const Size(1920, 1080));
    final (tv, phone) = await _pair();
    final onPhone = _frames(t, phone);
    final s = await demoServices(sync: tv);
    s.locale.value = const Locale('ru');
    await t.pumpWidget(
      testApp(s, formFactor: FormFactor.tv, section: Section.search),
    );
    await settle(t);

    final field = find.byType(TextField).first;
    await t.tap(field);
    await settle(t, 6);
    final req = onPhone.singleWhere(
      (f) => f.action == RemoteInputAction.request,
    );
    expect(req.field, RemoteInputField.search);
    expect(req.secret, isFalse);
    expect(req.deviceName, 'Living room TV');

    // The phone opens its sheet and types: the TV shows who types and the
    // text lands in its field (and runs the search).
    await phone.sendInput(
      req.session,
      to: 'ktv',
      action: RemoteInputAction.open,
    );
    await phone.sendInput(
      req.session,
      to: 'ktv',
      action: RemoteInputAction.text,
      text: 'Harbor',
    );
    await settle(t, 6);
    expect(t.widget<TextField>(field).controller!.text, 'Harbor');
    expect(find.text(ru.remoteInputNoticeTitle('Pixel 8')), findsOneWidget);
    expect(s.notices.current?.kind, InAppNoticeKind.remoteInput);
    await capture(t, '01-tv-search-typing');

    // The remote moves the focus off the field: the phone's input closes.
    FocusManager.instance.primaryFocus?.unfocus();
    await settle(t, 4);
    expect(
      onPhone.where(
        (f) => f.session == req.session && f.action == RemoteInputAction.cancel,
      ),
      hasLength(1),
    );
    expect(s.notices.contains(remoteInputNoticeId), isFalse);
  });

  testWidgets('TV: a password field asks for secret input; submit runs the '
      'field action; a cancel from the phone ends it', (t) async {
    final (tv, phone) = await _pair();
    final onPhone = _frames(t, phone);
    final notices = InAppNotices();
    String? submitted;
    final password = FocusNode();
    addTearDown(password.dispose);
    await t.pumpWidget(
      _harness(
        sync: tv,
        notices: notices,
        tv: true,
        home: Scaffold(
          body: Column(
            children: [
              const TextField(
                decoration: InputDecoration(
                  labelText: 'Address',
                ), // l10n-ignore
                keyboardType: TextInputType.url,
              ),
              TextField(
                focusNode: password,
                obscureText: true,
                decoration: const InputDecoration(
                  labelText: 'Password', // l10n-ignore
                ),
                onSubmitted: (v) => submitted = v,
              ),
            ],
          ),
        ),
      ),
    );
    password.requestFocus();
    await settle(t, 6);
    final req = onPhone.single;
    expect(req.field, RemoteInputField.password);
    expect(req.secret, isTrue);
    expect(req.label, 'Password');
    expect(req.text, isNull);
    await phone.sendInput(
      req.session,
      to: 'ktv',
      action: RemoteInputAction.submit,
      text: 'hunter2',
      secret: true,
    );
    await settle(t, 2);
    expect(submitted, 'hunter2');
    expect(notices.contains(remoteInputNoticeId), isFalse);

    // Focus again: a new session; the phone cancels it.
    password.unfocus();
    await settle(t, 2);
    password.requestFocus();
    await settle(t, 6);
    final again = onPhone.lastWhere(
      (f) => f.action == RemoteInputAction.request,
    );
    expect(again.session, isNot(req.session));
    await phone.sendInput(
      again.session,
      to: 'ktv',
      action: RemoteInputAction.open,
    );
    await settle(t, 2);
    expect(notices.contains(remoteInputNoticeId), isTrue);
    await phone.sendInput(
      again.session,
      to: 'ktv',
      action: RemoteInputAction.cancel,
    );
    await settle(t, 2);
    expect(notices.contains(remoteInputNoticeId), isFalse);
    expect(password.hasFocus, isTrue, reason: 'the remote keeps working');
  });

  testWidgets('TV: walking past fields with the remote asks nothing', (
    t,
  ) async {
    final (tv, phone) = await _pair();
    final onPhone = _frames(t, phone);
    final a = FocusNode();
    final b = FocusNode();
    addTearDown(a.dispose);
    addTearDown(b.dispose);
    await t.pumpWidget(
      _harness(
        sync: tv,
        notices: InAppNotices(),
        tv: true,
        home: Scaffold(
          body: Column(
            children: [
              TextField(focusNode: a),
              TextField(focusNode: b),
              const TextButton(onPressed: null, child: Text('x')),
            ],
          ),
        ),
      ),
    );
    a.requestFocus();
    await t.pump(const Duration(milliseconds: 100));
    b.requestFocus();
    await t.pump(const Duration(milliseconds: 100));
    b.unfocus();
    await settle(t, 6);
    expect(onPhone, isEmpty);
  });

  testWidgets('phone: the sheet opens by itself, sends as you type, and '
      'closes when the TV cancels', (t) async {
    setSurface(t, const Size(390, 844), dpr: 2);
    final (tv, phone) = await _pair();
    final onTv = _frames(t, tv);
    final s = await demoServices(sync: phone);
    s.locale.value = const Locale('ru');
    await t.pumpWidget(testApp(s, formFactor: FormFactor.phone));
    await settle(t);

    final session = await tv.requestInput(
      field: RemoteInputField.search,
      label: 'Поиск',
      text: 'Har',
    );
    await settle(t, 6);
    final input = find.byKey(const ValueKey('remote-input-field'));
    expect(input, findsOneWidget);
    expect(find.text(ru.remoteInputSheetTitle('Living room TV')), findsOne);
    expect(t.widget<TextField>(input).controller!.text, 'Har');
    expect(
      onTv.where((f) => f.action == RemoteInputAction.open).single.deviceName,
      'Pixel 8',
    );
    await t.enterText(input, 'Harbor Lights');
    await settle(t, 2);
    expect(onTv.last.action, RemoteInputAction.text);
    expect(onTv.last.text, 'Harbor Lights');
    await capture(t, '02-phone-sheet-search');

    // The TV's field lost focus: the sheet goes away.
    await tv.sendInput(session!, action: RemoteInputAction.cancel);
    await settle(t, 6);
    expect(input, findsNothing);

    // A password: hidden, with the hint; Done sends it and closes.
    final pw = await tv.requestInput(
      field: RemoteInputField.password,
      label: 'Пароль',
      secret: true,
    );
    await settle(t, 6);
    expect(t.widget<TextField>(input).obscureText, isTrue);
    expect(find.text(ru.remoteInputSecretHint), findsOneWidget);
    await t.enterText(input, 'hunter2');
    await settle(t, 2);
    await capture(t, '03-phone-sheet-password');
    await t.tap(find.byKey(const ValueKey('remote-input-done')));
    await settle(t, 6);
    expect(input, findsNothing);
    final last = onTv.last;
    expect(last.session, pw);
    expect(last.action, RemoteInputAction.submit);
    expect(last.text, 'hunter2');
    expect(last.secret, isTrue);
  });

  testWidgets('phone: closing the sheet cancels on the TV', (t) async {
    final (tv, phone) = await _pair();
    final onTv = _frames(t, tv);
    await t.pumpWidget(
      _harness(
        sync: phone,
        notices: InAppNotices(),
        tv: false,
        home: const Scaffold(),
      ),
    );
    final session = await tv.requestInput(field: RemoteInputField.text);
    await settle(t, 6);
    await t.tap(find.byKey(const ValueKey('remote-input-cancel')));
    await settle(t, 6);
    expect(find.byKey(const ValueKey('remote-input-field')), findsNothing);
    expect(onTv.last.session, session);
    expect(onTv.last.action, RemoteInputAction.cancel);
  });

  testWidgets('phone in the background: a notification; back in front: the '
      'sheet', (t) async {
    final (tv, phone) = await _pair();
    final notifier = _FakeNotifier();
    await t.pumpWidget(
      _harness(
        sync: phone,
        notices: InAppNotices(),
        tv: false,
        notifier: notifier,
        home: const Scaffold(),
      ),
    );
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    await t.pump();
    final session = await tv.requestInput(
      field: RemoteInputField.address,
      label: 'Адрес',
    );
    await settle(t, 4);
    expect(notifier.shown, [
      '${ru.remoteInputNotificationTitle} | '
          '${ru.remoteInputNotificationBody('Living room TV', 'Адрес')}',
    ]);
    expect(find.byKey(const ValueKey('remote-input-field')), findsNothing);

    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await settle(t, 6);
    expect(find.byKey(const ValueKey('remote-input-field')), findsOneWidget);
    expect(notifier.cancels, 1);
    expect(notifier.asks, 1, reason: 'asks while the user looks');

    // Gone before the user came back: nothing opens.
    await tv.sendInput(session!, action: RemoteInputAction.cancel);
    await settle(t, 6);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    await t.pump();
    final other = await tv.requestInput(field: RemoteInputField.text);
    await settle(t, 2);
    await tv.sendInput(other!, action: RemoteInputAction.cancel);
    await settle(t, 2);
    expect(notifier.cancels, 2);
    t.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await settle(t, 6);
    expect(find.byKey(const ValueKey('remote-input-field')), findsNothing);
  });
}
