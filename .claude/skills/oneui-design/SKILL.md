---
name: oneui-design
description: VyomaConnect's visual and motion language (One UI / ColorOS / iOS inspired) and the designsystem module's building blocks. Read before creating or changing any screen, component, widget, tile or notification visual.
---

# Vyoma design rules

**Never purple by default.** No generic AI gradients. The look is One UI / ColorOS / iOS 27: glass materials over a
soft accent-tinted canvas, colorful feature icons, a floating glass tab bar. **Performance is a requirement**: no
per-frame blur except on chrome that content scrolls under, and measure with `dumpsys gfxinfo` after visual changes.

## Glass (core/designsystem/Glass.kt)
- Canvas: `Modifier.aurora()` (theme background + two cached accent glows; faint in pure-black mode).
- Cards: `Modifier.glassCard(shape)` = static glass (translucent `Glass.fill` + `Glass.edge` highlight). No blur, cheap.
  Buttons on glass use `Glass.raised`. `SectionCard` already does this. Don't use `surfaceContainerLowest` for cards.
- Real blur only for chrome: record content with `backdropSource(backdrop)` (put it OUTSIDE `aurora()` in the
  modifier chain so the canvas is recorded too), then `glassChrome(backdrop, shape)` on the bar, which must not be
  inside the source. Gate chrome that comes and goes in the draw phase (`drawWithContent { if (shown) drawContent() }`),
  not with `if` in composition: composing it on every threshold crossing costs a frame and a fresh layer.
- Liquid glass (Android 13+, `liquidGlassSupported`): `glassChrome(..., bevel = 16.dp)` on pills/circles runs one AGSL
  pass (`LiquidGlass`) over a lightly blurred (8 dp) layer recorded with a margin around the shape: the rim samples
  outward, so what's just outside bends in at the edge; slight colour split; specular rim lit from the top left; tint
  baked in (no border). Dark mode is clear (half tint); light mode keeps more milk (0.85) for legibility over photos
  and adds a faint shaded underside (`shade`), since a white highlight vanishes on white. `Modifier.liquidLens(backdrop, strength, zoom, rect)` draws a clear
  magnifying drop over an element's own drawing plus the backdrop around it, so it can spill past the edges; the tab
  bar's selection lifts into one while dragged and briefly on each switch. Keep dispersion subtle (≤ 0.15): more
  reads as a rainbow glitch. AGSL builtins (`refract`, `reflect`, …) can't be uniform names. Below 13: frosted fallback.
- Feature colors: `SystemColor` (iOS system palette, light/dark tones, no indigo/purple). `IconBadge` is a filled
  squircle with a white glyph; give each feature a stable color (notifications RED, messages GREEN, clipboard ORANGE,
  files/connection BLUE, touchpad TEAL, find YELLOW, battery GREEN, destructive RED, info/neutral GRAY).
- Control Center tiles: colored glyph on `Glass.raised`; toggles fill with their color (white glyph) while on.
- Root navigation: `TabsRoot` (app/ui/TabBar.kt): Home / Clipboard / Settings in a glass pill with a sliding lens,
  touchpad as a separate round glass button. Lists get `LocalBottomBarInset` padding automatically via the scaffold.

## Theme (core/designsystem)
- `VyomaTheme(ThemeSettings)`: palettes `DYNAMIC` (wallpaper accent, neutral surfaces), `OCEAN`, `GRAPHITE`, `EMBER`,
  `MINT`, `SAND`; DarkMode SYSTEM/LIGHT/DARK; `amoled` = true black.
- Canvas `aurora()`, cards `glassCard`, raised chips `Glass.raised`; dialogs keep opaque `surfaceContainerHigh`.
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
