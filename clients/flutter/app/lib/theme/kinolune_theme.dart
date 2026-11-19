// SPDX-License-Identifier: MPL-2.0
//
// The whole visual language of the app lives in this one file. Values come
// from the generated design tokens in `design/flutter/pa_design` (DSN-002:
// PaColorScheme, PaTokens, PaDensity, Inter font); this file only adapts them
// to Flutter's ThemeData and to the form factors of the app.

import 'package:material_ui/material_ui.dart';
import 'package:pa_design/pa_design.dart';

import '../platform/form_factor.dart';

const _c = PaColorScheme.dark;

/// Colour roles used by the screens (dark theme, 01 §4.5).
abstract final class KColors {
  static final bgBase = _c.bgBase;
  static final bgElevated = _c.bgElevated;
  static final bgElevated2 = _c.bgElevated2;
  static final stroke = _c.borderSubtle;
  static final textPrimary = _c.textPrimary;
  static final textSecondary = _c.textSecondary;

  /// UI-021: 47 % white instead of the design token's 38 % (0x61), which
  /// had 3.5:1 on the backgrounds; this is at least 4.68:1 (WCAG AA for
  /// small text). The token itself belongs to the design system (DSN).
  static const textTertiary = Color(0x78FFFFFF);

  /// [textSecondary] as an opaque colour over [bgBase], for text drawn over
  /// artwork (UI-021): a translucent grey takes on the picture's colours.
  static const textSecondaryOnArt = Color(0xFFA7A7A8);

  static const accent = PaTokens.accentDefault;
  static final accentSoft = _c.textLink;
  static const live = PaTokens.accentLive;

  /// TV focused button: white fill, dark label (01 §4.5 focus/*).
  static final focusFill = _c.focusFill;
  static final focusLabel = _c.focusLabel;
  static final focusGlow = _c.focusRing.withValues(alpha: .45);

  static const scrimStrong = PaTokens.scrimBottomStrong;

  /// Green, yellow and red of statuses (UI-044 «Всё ли в порядке»).
  static final statusOk = _c.statusSuccess;
  static final statusWarning = _c.statusWarning;
  static final statusProblem = _c.statusDanger;
}

/// Motion tokens (01 §4.7).
abstract final class KMotion {
  static const focus = PaTokens.motionFocusOutDuration;
  static const focusOut = Duration(milliseconds: 150);
  static final press = PaTokens.motionPress.duration;
  static final page = PaTokens.motionPage.duration;
  static final crossfadeArt = PaTokens.motionCrossfadeArt.duration;
  static const focusCurve = Curves.easeOutBack;
  static final pageCurve = PaTokens.motionPage.curve;
}

/// Size/density per form factor from `PaDensity`. Android TV reports about
/// 960x540 logical pixels, while TV tokens are in 1920x1080 points, so TV
/// values are scaled by 1/2.
@immutable
class KDensity {
  const KDensity({
    required this.formFactor,
    required this.pageInsets,
    required this.gap,
    required this.rowGap,
    required this.posterWidth,
    required this.landscapeWidth,
    required this.albumWidth,
    required this.cardRadius,
    required this.panelRadius,
    required this.focusScalePoster,
    required this.focusScaleCard,
    required this.focusScaleButton,
    required this.type,
  });

  factory KDensity._from(FormFactor f, PaDensity p, double k) {
    double s(TextStyle t) => t.fontSize! * k;
    final tv = f == FormFactor.tv;
    final touch = f == FormFactor.phone;
    return KDensity(
      formFactor: f,
      pageInsets: EdgeInsets.fromLTRB(
        p.layoutMargin * k,
        tv ? 16 : (touch ? 12 : 24),
        p.layoutMargin * k,
        tv ? p.layoutSafeBottom * k : 24,
      ),
      gap: p.layoutGutter * k,
      rowGap: p.layoutSectionGap * k,
      posterWidth: touch ? 112 : p.layoutPosterWidth * k,
      landscapeWidth: p.layoutCardWidth * k,
      albumWidth: (p.layoutPosterWidth * k * 1.05).roundToDouble(),
      cardRadius: p.cardRadius * k,
      panelRadius: p.shapePanel * k,
      focusScalePoster: touch
          ? 1
          : (tv ? PaTokens.posterFocusScale : PaTokens.posterHoverScale),
      focusScaleCard: touch
          ? 1
          : (tv ? PaTokens.cardFocusScale : PaTokens.focusScaleHover),
      focusScaleButton: touch ? 1 : PaTokens.buttonFocusScale,
      type: KType(
        display: s(p.typeDisplay),
        title1: s(p.typeTitle1),
        title2: s(p.typeTitle2),
        headline: s(p.typeHeadline),
        body: s(p.typeBody),
        callout: s(p.typeCallout),
        caption: s(p.typeCaption),
        caption2: s(p.typeCaption2),
      ),
    );
  }

  final FormFactor formFactor;
  final EdgeInsets pageInsets;
  final double gap;
  final double rowGap;
  final double posterWidth;
  final double landscapeWidth;
  final double albumWidth;
  final double cardRadius;
  final double panelRadius;
  final double focusScalePoster;
  final double focusScaleCard;
  final double focusScaleButton;
  final KType type;

  bool get isTv => formFactor == FormFactor.tv;
  bool get isPhone => formFactor == FormFactor.phone;
  bool get isDesktop => formFactor == FormFactor.desktop;

  /// Top padding of a shell page: [pageInsets] plus the system inset above
  /// the page (status bar and display cutout when Android draws edge to
  /// edge, the notch on iOS). Inside a [SafeArea] and on desktop and TV the
  /// system inset is zero, so only [pageInsets] remains. Page headers and
  /// heroes use this instead of `pageInsets.top`; backdrops still bleed
  /// under the status bar.
  double safeTop(BuildContext context) =>
      pageInsets.top + MediaQuery.paddingOf(context).top;

  static KDensity of(BuildContext context) =>
      forFactor(FormFactorScope.of(context));

  static KDensity forFactor(FormFactor f) => switch (f) {
    FormFactor.tv => tv,
    FormFactor.phone => phone,
    FormFactor.desktop => desktop,
  };

  static final desktop = KDensity._from(
    FormFactor.desktop,
    PaDensity.desktop,
    1,
  );
  static final phone = KDensity._from(FormFactor.phone, PaDensity.phone, 1);
  static final tv = KDensity._from(FormFactor.tv, PaDensity.tv, .5);
}

@immutable
class KType {
  const KType({
    required this.display,
    required this.title1,
    required this.title2,
    required this.headline,
    required this.body,
    required this.callout,
    required this.caption,
    required this.caption2,
  });
  final double display,
      title1,
      title2,
      headline,
      body,
      callout,
      caption,
      caption2;
}

/// Inter Variable (OFL-1.1) bundled by `pa_design`; system fonts as fallback.
const kFontFamily = 'packages/pa_design/Inter';
const kFontFallback = <String>[
  'Roboto',
  'Noto Sans',
  'Cantarell',
  'DejaVu Sans',
];

ThemeData buildKinoluneTheme(FormFactor formFactor) {
  final d = KDensity.forFactor(formFactor);
  final t = d.type;
  final scheme = ColorScheme.dark(
    primary: KColors.accent,
    onPrimary: Colors.white,
    secondary: KColors.accentSoft,
    surface: KColors.bgBase,
    surfaceContainerLowest: KColors.bgBase,
    surfaceContainerLow: KColors.bgElevated,
    surfaceContainer: KColors.bgElevated,
    surfaceContainerHigh: KColors.bgElevated2,
    surfaceContainerHighest: KColors.bgElevated2,
    onSurface: KColors.textPrimary,
    onSurfaceVariant: KColors.textSecondary,
    outline: KColors.stroke,
    outlineVariant: KColors.stroke,
    error: KColors.live,
  );
  TextStyle s(double size, FontWeight w, {double? height, Color? color}) =>
      TextStyle(
        fontSize: size,
        fontWeight: w,
        height: height,
        color: color ?? KColors.textPrimary,
        letterSpacing: 0,
      );
  final textTheme = TextTheme(
    displayLarge: s(t.display, FontWeight.w700, height: 1.15),
    displayMedium: s(t.display * .85, FontWeight.w700, height: 1.15),
    headlineLarge: s(t.title1, FontWeight.w700, height: 1.2),
    headlineMedium: s(t.title2, FontWeight.w600, height: 1.25),
    headlineSmall: s(t.title2 * .9, FontWeight.w600, height: 1.25),
    titleLarge: s(t.headline, FontWeight.w600, height: 1.3),
    titleMedium: s(t.callout, FontWeight.w600, height: 1.3),
    titleSmall: s(t.caption, FontWeight.w600, height: 1.3),
    bodyLarge: s(t.body + 1, FontWeight.w400, height: 1.45),
    bodyMedium: s(t.body, FontWeight.w400, height: 1.45),
    bodySmall: s(
      t.caption,
      FontWeight.w400,
      height: 1.35,
      color: KColors.textSecondary,
    ),
    labelLarge: s(t.callout, FontWeight.w600),
    labelMedium: s(t.caption, FontWeight.w500, color: KColors.textSecondary),
    labelSmall: s(t.caption2, FontWeight.w500, color: KColors.textSecondary),
  );

  return ThemeData(
    useMaterial3: true,
    brightness: Brightness.dark,
    colorScheme: scheme,
    scaffoldBackgroundColor: KColors.bgBase,
    canvasColor: KColors.bgBase,
    fontFamily: kFontFamily,
    fontFamilyFallback: kFontFallback,
    textTheme: textTheme,
    visualDensity: VisualDensity.standard,
    splashFactory: formFactor == FormFactor.tv
        ? NoSplash.splashFactory
        : InkSparkle.splashFactory,
    focusColor: Colors.white.withValues(alpha: .12),
    hoverColor: Colors.white.withValues(alpha: .06),
    dividerTheme: DividerThemeData(
      color: KColors.stroke,
      thickness: 1,
      space: 1,
    ),
    appBarTheme: AppBarTheme(
      backgroundColor: Colors.transparent,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      titleTextStyle: textTheme.titleLarge,
    ),
    cardTheme: CardThemeData(
      color: KColors.bgElevated,
      elevation: 0,
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(d.cardRadius),
      ),
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: KColors.bgElevated,
      surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(d.panelRadius),
      ),
      titleTextStyle: textTheme.headlineSmall,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: Colors.white.withValues(alpha: .06),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide.none,
      ),
      // UI-021: the light accent, the deep one has 3.3:1 on the field.
      focusedBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(color: KColors.accentSoft, width: 2),
      ),
      labelStyle: textTheme.bodyMedium?.copyWith(color: KColors.textSecondary),
      hintStyle: textTheme.bodyMedium?.copyWith(color: KColors.textTertiary),
    ),
    chipTheme: ChipThemeData(
      backgroundColor: Colors.white.withValues(alpha: .06),
      selectedColor: Colors.white,
      labelStyle: textTheme.labelLarge?.copyWith(fontSize: t.caption + 1),
      secondaryLabelStyle: textTheme.labelLarge?.copyWith(
        fontSize: t.caption + 1,
        color: Colors.black,
      ),
      side: BorderSide.none,
      shape: const StadiumBorder(),
      showCheckmark: false,
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: KColors.bgElevated.withValues(alpha: .96),
      indicatorColor: KColors.accent.withValues(alpha: .22),
      surfaceTintColor: Colors.transparent,
      labelTextStyle: WidgetStatePropertyAll(textTheme.labelSmall),
      height: 64,
    ),
    // UI-021: text buttons take the light accent (textLink, 10:1 on the
    // background); the deep accent (primary) has only 3.7:1 as text.
    // UI-021: the caret in the light accent (3.3:1 in the deep one).
    textSelectionTheme: TextSelectionThemeData(
      cursorColor: KColors.accentSoft,
      selectionHandleColor: KColors.accentSoft,
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(foregroundColor: KColors.accentSoft),
    ),
    progressIndicatorTheme: const ProgressIndicatorThemeData(
      color: KColors.accent,
      linearTrackColor: Color(0x33FFFFFF),
    ),
    snackBarTheme: SnackBarThemeData(
      backgroundColor: KColors.bgElevated2,
      // The raw text theme has no family (ThemeData.fontFamily only goes
      // into its own text theme): the message was in the fallback font.
      contentTextStyle: textTheme.bodyMedium?.copyWith(
        fontFamily: kFontFamily,
        fontFamilyFallback: kFontFallback,
      ),
      behavior: SnackBarBehavior.floating,
    ),
    listTileTheme: ListTileThemeData(
      iconColor: KColors.textSecondary,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
    ),
    scrollbarTheme: ScrollbarThemeData(
      thumbColor: WidgetStatePropertyAll(Colors.white.withValues(alpha: .18)),
    ),
    // Platforms not listed used to fall back to the zoom transition; they
    // are listed now so that Reduce Motion covers them too.
    pageTransitionsTheme: const PageTransitionsTheme(
      builders: {
        TargetPlatform.android: ReduceMotionPageTransitionsBuilder(
          FadeForwardsPageTransitionsBuilder(),
        ),
        TargetPlatform.linux: ReduceMotionPageTransitionsBuilder(
          FadeForwardsPageTransitionsBuilder(),
        ),
        TargetPlatform.iOS: ReduceMotionPageTransitionsBuilder(
          ZoomPageTransitionsBuilder(),
        ),
        TargetPlatform.macOS: ReduceMotionPageTransitionsBuilder(
          ZoomPageTransitionsBuilder(),
        ),
        TargetPlatform.windows: ReduceMotionPageTransitionsBuilder(
          ZoomPageTransitionsBuilder(),
        ),
        TargetPlatform.fuchsia: ReduceMotionPageTransitionsBuilder(
          ZoomPageTransitionsBuilder(),
        ),
      },
    ),
  );
}

/// Reduce Motion (UI-021, 01 §4.7): when the system asks to remove
/// animations (`MediaQuery.disableAnimations`: "Remove animations" on
/// Android, "Reduce Motion" on iOS and macOS, "Show animations" off on
/// Windows, GNOME's "Reduce animation"), a page fades in over the previous
/// one instead of sliding or zooming; otherwise [base] is used.
class ReduceMotionPageTransitionsBuilder extends PageTransitionsBuilder {
  const ReduceMotionPageTransitionsBuilder(this.base);

  final PageTransitionsBuilder base;

  static bool _reduced(BuildContext context) =>
      MediaQuery.maybeDisableAnimationsOf(context) ?? false;

  @override
  Duration get transitionDuration => base.transitionDuration;

  @override
  Duration get reverseTransitionDuration => base.reverseTransitionDuration;

  @override
  DelegatedTransitionBuilder? get delegatedTransition {
    final delegated = base.delegatedTransition;
    if (delegated == null) return null;
    return (context, animation, secondaryAnimation, allowSnapshotting, child) =>
        _reduced(context)
        ? child
        : delegated(
            context,
            animation,
            secondaryAnimation,
            allowSnapshotting,
            child,
          );
  }

  @override
  Widget buildTransitions<T>(
    PageRoute<T> route,
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
    Widget child,
  ) {
    if (!_reduced(context)) {
      return base.buildTransitions(
        route,
        context,
        animation,
        secondaryAnimation,
        child,
      );
    }
    return FadeTransition(
      key: const ValueKey('reduced-motion-transition'),
      opacity: CurvedAnimation(parent: animation, curve: Curves.easeOut),
      child: child,
    );
  }
}
