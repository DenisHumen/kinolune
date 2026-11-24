// SPDX-License-Identifier: MPL-2.0
//
// CPU benchmark entry point of the Twitch chat (TWI-012): the chat panel
// alone, fed by the core's made-up chat (local images, no network).
//
//   flutter build apk --profile -t lib/features/twitch/chat/chat_bench_main.dart \
//     --dart-define=KINOLUNE_CHAT_BENCH_RATE=100
//
// KINOLUNE_CHAT_BENCH_PAUSE=true starts with animated emotes paused;
// KINOLUNE_FORM_FACTOR=tv measures the TV density with thinning;
// KINOLUNE_CHAT_BENCH_THIRD_PARTY=false leaves out the made-up 7TV, BTTV
// and FFZ emotes (TWI-023), which are on by default like in the app.
// tools: clients/flutter/app/tool/chat_cpu_bench.sh.

import 'package:material_ui/material_ui.dart';
import 'package:pa_library/pa_library.dart' show RustLibrary;

import '../../../config.dart';
import '../../../l10n/l10n.dart';
import '../../../platform/form_factor.dart';
import '../../../theme/kinolune_theme.dart';
import 'chat_panel.dart';
import 'core_chat_feed.dart';

const _rate = int.fromEnvironment(
  'KINOLUNE_CHAT_BENCH_RATE',
  defaultValue: 100,
);
const _pause = bool.fromEnvironment('KINOLUNE_CHAT_BENCH_PAUSE');
const _thirdParty = bool.fromEnvironment(
  'KINOLUNE_CHAT_BENCH_THIRD_PARTY',
  defaultValue: true,
);

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await RustLibrary.ensureInitialized();
  final formFactor = AppConfig.forcedFormFactor ?? FormFactor.phone;
  runApp(_BenchApp(formFactor: formFactor));
}

class _BenchApp extends StatelessWidget {
  const _BenchApp({required this.formFactor});

  final FormFactor formFactor;

  @override
  Widget build(BuildContext context) => FormFactorScope(
    formFactor: formFactor,
    child: MaterialApp(
      debugShowCheckedModeBanner: false,
      supportedLocales: AppLocalizations.supportedLocales,
      localizationsDelegates: kLocalizationsDelegates,
      theme: buildKinoluneTheme(formFactor),
      home: const _BenchPage(),
    ),
  );
}

class _BenchPage extends StatefulWidget {
  const _BenchPage();

  @override
  State<_BenchPage> createState() => _BenchPageState();
}

class _BenchPageState extends State<_BenchPage> {
  CoreChatFeed? _feed;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_feed != null) return;
    chatOptions(context, animate: !_pause, thirdPartyEmotes: _thirdParty).then((
      o,
    ) {
      if (mounted) setState(() => _feed = CoreChatFeed.demo(_rate, o));
    });
  }

  @override
  Widget build(BuildContext context) {
    final feed = _feed;
    return Scaffold(
      backgroundColor: KColors.bgBase,
      body: SafeArea(
        child: feed == null
            ? const SizedBox.shrink()
            : TwitchChatPanel(feed: feed, pauseAnimations: _pause),
      ),
    );
  }
}
