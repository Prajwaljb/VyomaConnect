---
name: oneui-design
description: VyomaConnect's visual and motion language (One UI / ColorOS / iOS inspired) and the designsystem module's building blocks. Read before creating or changing any screen, component, widget, tile or notification visual.
---

# Vyoma design rules

**Never purple by default.** No generic AI gradients. The look is One UI / ColorOS / iOS 27: glass materials over a
soft accent-tinted canvas, colorful feature icons, a floating glass tab bar. **Performance is a requirement**: no
per-frame blur except on chrome that content scrolls under, and measure with `dumpsys gfxinfo` after visual changes.

## Glass (core/designsystem/Glass.kt)
- Canvas: `Modifier.aurora()` (theme background + two cached accent glows; in pure black a tighter glow that
  leaves the bottom true black). No decorative textures (stars were tried and rejected: too specific).
- Depth tokens: `glassDepth` (`GlassDepth` Dark / Amoled / Light) drive every surface so the themes stay consistent.
- Cards: `Modifier.glassCard(shape, fill?)` = a depth slab: soft shadow outside only, top-lit gradient fill, sheen,
  lit→shaded rim. No blur, cheap (all `drawWithCache`). `SectionCard`/`glassListGroup` use it. Never flat fills.
- Anything pressable on a card (buttons, chips, keys, tiles, PillButton): `Modifier.glassRaised(shape, color?)`, a
  smaller lit bead; pass `color` for accent/tinted or for chips over artwork (dark translucent).
- Real blur only for chrome: record content with `backdropSource(backdrop)` (put it OUTSIDE `aurora()` in the
  modifier chain so the canvas is recorded too), then `glassChrome(backdrop, shape)` (AGSL liquid lens on 33+:
  `LiquidGlass`/`liquidLens`: physically based, like Telegram's/Apple's: Snell refraction (n≈1.5) through a
  quarter-circle rim (≤ 1/5 of the element) over a slab 6 rims deep, so the rim shows what's ACROSS the element and
  beyond it (the recorded margin), bent like water. Rejected: plain outward bend, weak inward/convex bend). Every
  chrome element uses the tab bar's values: `blur = 8.dp, bevel = 16.dp`. The bar must not be inside
  the source. Compose/draw chrome only while visible.
- Header (`CollapsingHeaderScaffold`): no slab. A soft status-bar fade; back in a 48 dp liquid circle; the title
  appears in a liquid capsule once collapsed (> 0.6); actions share one capsule. Same blur/bevel as the tab bar.
- Dialogs: `GlassDialog` (same API as AlertDialog): opaque `surfaceContainerHigh` slab with the card rim. Never
  Material `AlertDialog`.
- Value bars: `LiquidSlider` (thumbless capsule that swells under the finger, relative drag) for scrubbers and
  in-card volume; the big volume control is `Dashboard.VolumeBar`: a glass tube (`LensStyle.Tube` lens over the liquid
  only, never over text: refracted text reads as a glitch) with a lit meniscus, a `LensStyle.Drop` drop riding the
  surface while held, `draggable` relative drag, rubber-band ends, labels drawn twice and the second copy clipped
  to the liquid. `LiquidSlider` grows the same drop while held; pass `underlay` (the surface colour under it).
- Press glass: `pressable(glass = true)` / `pressScale(1f, glass = true)` turns capsules and circles into a clear
  drop while held. Put it BEFORE the background modifiers so the drop sees them.
- `liquidLens` is a Modifier.Node that allocates its layers only on first visible frame: cheap on every control.
- Icon buttons: `BounceIconButton` (never Material `IconButton`); glass circles/capsules put `pressScale` BEFORE
  `glassChrome` so the glass itself squishes, not just the glyph.
- Feature colors: `SystemColor` (iOS system palette, light/dark tones, no indigo/purple). `IconBadge` is a filled
  squircle with a white glyph; give each feature a stable color (notifications RED, messages GREEN, clipboard ORANGE,
  files/connection BLUE, touchpad TEAL, find YELLOW, battery GREEN, destructive RED, info/neutral GRAY).
- Control Center tiles: colored glyph on `glassRaised`; toggles fill with their color (white glyph) while on.
- Root navigation: `TabsRoot` (app/ui/TabBar.kt): Home / Clipboard / Settings in a glass pill with a sliding lens,
  touchpad as a separate round glass button. Tabs stay composed (keep-alive) and are only placed when shown.
  Lists get `LocalBottomBarInset` padding automatically via the scaffold.

## Theme (core/designsystem)
- Type: bundled Inter variable (`res/font/inter.ttf`, OFL in `core/designsystem/licenses`): display optical size
  for title/headline/display styles, text cut below; tight negative tracking on large sizes; `tnum` for live numbers.
- `VyomaTheme(ThemeSettings)`: palettes `DYNAMIC` (wallpaper accent, neutral surfaces), `OCEAN`, `GRAPHITE`, `EMBER`,
  `MINT`, `SAND`; DarkMode SYSTEM/LIGHT/DARK; `amoled` = true black.
- Canvas `aurora()`, cards `glassCard`, raised chips `glassRaised`, dialogs `GlassDialog`.
- Status colors via `Status.connected/pending/offline(dark)`, the same in every palette.
- Shapes: cards `large` (26 dp), hero cards `extraLarge` (32 dp), chips/icons `medium` (20 dp).

## Layout
- Every top-level screen uses `CollapsingHeaderScaffold`: a tall hero title (~30% of screen height) that keeps content in
  thumb reach, collapsing into a compact bar. 16 dp side gutters, 12 dp between cards.
- Long or unbounded groups (folder listings) are one lazy item per row: draw the card once behind them with
  `listModifier = Modifier.glassListGroup(listState, contentType)` on `CollapsingHeaderScaffold` and cancel the list
  gap between rows. Never give each row its own glass: every slice gets its own edge highlight. No per-row
  `animateItem` there either; the card has to move as one.
- Group related rows in `SectionCard` with `ListRow`s (`iconTint = SystemColor.X.color` → colored squircle `IconBadge`,
  dividers inset past the icon).
- Section titles use `SectionLabel` (small, muted, drawn low so it belongs to the card below, not the gap above).
- Order by frequency: glanceable state first, then what you adjust (media, volume), then one-shot actions.
- One job per button, named for what it does ("Send", "PC files", "Upload here"), never one button whose meaning
  changes with state. No two buttons on the same screen open the same thing. When a feature appears only
  sometimes (companion online), swap it into a fixed slot so the other buttons never move.
- One clear primary action per card. Touch targets ≥ 48 dp; keep the bottom 70 px clear of the floating tab bar
  (`LocalBottomBarInset`).
- Controls: `PillButton` (prominent = accent fill, else tinted), `SegmentedControl` for 2–4 exclusive choices,
  `VyomaSwitch` for on/off. No Material `Button`/`FilledTonalButton`/`Switch`/`SegmentedButton` outside dialogs.

## Motion
- Springs only (`VyomaMotion`): `spatial()` movement (slight overshoot), `effects()` alpha/color (never bounces),
  `release()` anything let go of (iOS wobble), `page()` navigation (no overshoot), `leading()/trailing()` pairs for
  liquid shapes (lens, switch thumb): the edge moving toward the target leads, the other trails, so it stretches.
- Respond on touch-down, not on tap-up: `pressable` / `pressScale` dip immediately (Modifier.Node, layer-only) and
  bounce on release like water: width settles on `release()`, height on the softer `jelly()`, so it wobbles out of
  step instead of just scaling. If a scroll steals the gesture they ease back without the bounce.
- Things that hit a limit stretch (rubber band, diminishing) instead of stopping dead: see the volume bar.
- Navigation: push slides the page in from the right over a parallaxed parent; back follows the thumb through
  `SeekableTransitionState` (linear timeline while seeking, spring on commit/cancel). Tabs drift 1/12 width in the
  direction travelled.
- Entrances: `Staggered(index)` once per item, 40 ms apart, never replayed on scroll-back; height lands on `jelly()`.
  Animated values are read only inside `graphicsLayer`, never returned to composition.
- Tabs stay composed once visited (others warmed up 800 ms after start), measured but unplaced while hidden; a
  switch is a layer-only drift + fade, so it never composes a screen in one frame.
- List changes: `Modifier.animateItem()`; size changes: `animateContentSize()`.
- Performance is a requirement: animate in draw/layer phases (`graphicsLayer {}`, `drawBehind {}`, `placeWithLayer`),
  read animated values inside those lambdas, use `derivedStateOf` for thresholds, and never run endless animations
  on a resting screen: anything under the glass chrome forces the blur to re-render every frame. The connection orb
  breathes 3 times on connect, then rests. Idle check: `gfxinfo reset`, wait 3 s, expect ~0 frames. Check `dumpsys gfxinfo` after motion work (target < 1% janky at 120 Hz).

## Haptics (`Haptic` in Motion.kt; always through `HapticFeedback.perform`)
Tap = buttons/cards · Key = keyboard-like keys · ToggleOn/ToggleOff = switches and toggle controls · Select = tabs,
segments · Detent = steps while dragging a value (every 10% volume, scroll-wheel notches) · Edge = end stops and
mode thresholds · Hold = long-press grab · Success/Failure = outcomes (sent, paired, declined). One haptic per action;
never haptics on passive updates.

## Widgets (Glance, Phase 1)
Same palette tokens; system widget corner radius; never show a spinner forever. Render cached state and an
"offline" look when the PC is away.

## Copy
Short, friendly, specific ("Connected via Wi-Fi", "Make sure this code matches on both screens"). No exclamation spam.

## Battery (background is the default state)
- Nothing ticks while the UI is hidden: tickers/progress clocks run under `repeatOnLifecycle(STARTED)`; no
  `Flow.sample()` (its fixed-period ticker wakes forever), use the throttle in `LinkService` (`conflate` + delay).
- Wi-Fi multicast lock only while `AppContainer.uiVisible`; otherwise rely on the PC's broadcasts and keep-alive.
- Widgets: skip identical snapshots, re-encode art only when it changes (WebP). Never poll when a flow can say it.
- Check: `/proc/<pid>/stat` ticks over 60 s in background (target ≲ 20 ticks), `dumpsys wifi` for locks.

## Rendering cost (glass makes every redraw expensive)
- Every chrome element replays the page behind it, so a page-level offscreen buffer is multiplied by the number of
  glass elements. Never fade pages/large content with a default layer alpha: use
  `compositingStrategy = CompositingStrategy.ModulateAlpha`, or draw a shade (push/pop dims the page below this way).
- Idle must be idle: periodic data (stats every 2 s) snaps small changes and glides big ones at ~30 fps by hand
  (`withFrameMillis` + `delay(33)`; `preferredFrameRate` is not honoured here); clocks tick on the second.
  Measure: `dumpsys gfxinfo` frames over 6 s at rest, and `framestats` (ui vs issue vs gpu) for transitions.
