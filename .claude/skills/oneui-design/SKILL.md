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
- Cards: `Modifier.glassCard(shape, fill?)` = iOS grouped surfaces: opaque, flat, high contrast (`#1C1C1E` on the dark
  and true-black canvas, white on light with a faint shadow). No gradient fill, no translucency (translucent rimmed
  panes were called cheap twice). Edge: a faint tilt-lit SWEEP hairline (`specularRim`; the user found a
  linear-gradient rim misaligned) plus, in the dark, a soft glow on the lit side baked INTO the fill as one radial
  gradient (a separate glow pass over the card cost 0.4 ms GPU/frame; in-fill is ~free). Measured ≈ +0.1–0.2 ms.
  Card slices (`cardSegment`) pass `lit = false`. Refraction belongs only to things that float.
- Anything pressable on a card (buttons, chips, keys, tiles, PillButton): `Modifier.glassRaised(shape, color?)`, a plain
  even fill plus a 0.9 dp specular hairline that follows the phone's tilt (a FIXED painted rim read as fake; the user
  asked for it back once it moved like iOS 26); pass `color` for accent/tinted or for chips over artwork (dark translucent).
- Light: `SpecularLight.angle` (Light.kt) is where the light is on screen (225° = top left at rest), driven by the
  gravity sensor only while the UI is started (`TrackSpecularLight` in `VyomaTheme`), smoothed, 1.5° dead band. Every
  specular edge uses it: `specularRim(...)` (chrome stroke, raised controls) and the AGSL `light` uniform (lens rims).
  Read it only in draw lambdas, never in composition. A still phone renders ~0 frames; measured no jank cost.
  Coloured solid objects (IconBadge, DeviceGlyph) take `Modifier.specularEdge(shape, width)` like iOS 26 icon edges.
  Tilt moves LIGHT only. A canvas parallax (aurora drifting up to 36 dp) was built and removed: unnoticeable behind
  such soft glows. Never move cards, text or controls with tilt (iOS doesn't inside apps).
- Real blur only for chrome: record content with `backdropSource(backdrop)` (put it OUTSIDE `aurora()` in the
  modifier chain so the canvas is recorded too), then `glassChrome(backdrop, shape)` (AGSL liquid lens on 33+:
  `LiquidGlass`/`liquidLens`: physically based, like Telegram's/Apple's: Snell refraction (n≈1.5) through a
  quarter-circle rim (≤ 1/5 of the element) over a slab 6 rims deep, so the rim shows what's ACROSS the element and
  beyond it (the recorded margin), bent like water. Rejected: plain outward bend, weak inward/convex bend).
  The user wants THIS Snell refraction (the Telegram-studied one): chrome rim shows what's across the element.
  A "wrap" variant that sampled outside the edge was tried and rejected ("you removed the refraction"). Knob,
  switch, segmented and tab drops use `Lift` (outward Snell, negative depth). Under a drop only the drop shows
  (`LensPainter` cuts the original out and crossfades it while the drop forms).
  Chrome rendering: one chained blur+refraction pass per element at quarter resolution (`CHROME_RES`), the
  shader's shape inflated 2 low-res px so the visible outline is a full-res anti-aliased clip, plus a full-res
  `specularRim` stroke (light mode: dark hairline base). Measured alternatives: full res +3 ms GPU; one shared
  full-page blur was slower (replays the whole page). Every
  chrome element uses the tab bar's values: `blur = 8.dp, bevel = 16.dp`. The bar must not be inside
  the source. Compose/draw chrome only while visible.
- Header (`CollapsingHeaderScaffold`): no slab. A soft status-bar fade; back in a 48 dp liquid circle; actions share
  one capsule. The title is ONE travelling Text (the hero copy is an invisible placeholder): it scrolls 1:1 with the
  page (no hero parallax, else it slid over the hero glyph), pins in the bar and shrinks to titleMedium; at collapse
  > 0.72 a glass bubble forms around it (height leads, width wobbles out with `jelly()`). The hero fades by 75%.
  A trailing "collapse-room" item lets any page that scrolls at all collapse fully (no half-faded stuck title).
  A scroll that stops mid-collapse snaps open/closed; the snap runs in its own child job (run inline, a finger
  interrupting it threw a cancellation that killed the watcher, and nothing snapped again).
- Dialogs: `GlassDialog` (same API as AlertDialog): opaque `surfaceContainerHigh` slab with the card rim. Never
  Material `AlertDialog`.
- Value bars (Apple's, `LiquidSlider`): with `knob` (default 24 dp) the iOS 26 slider: slim track, wide white
  capsule knob; `knob = null` is the Apple Music scrubber: bare track that thickens under the finger. Relative drag
  after horizontal slop. ONE knob throughout (continuity): at rest a drawn `frostedKnob` (no layers: a resting lens
  cost scroll frames); held, `liquidLens(style = LensStyle.Lift, frost = { 1 - swell })` of the same rect grows ~45%
  and clears; an inward rim mirrored the track into stripes. Volume (`Dashboard.VolumeBar`) is an iOS card: name +
  % over the knobbed slider between quiet/loud speakers. The media `Scrubber` drives one `Animatable` progress (small
  drifts snap, jumps glide) and holds our seek target until MPRIS agrees (browsers report stale/zero after a seek).
- Switches (`VyomaSwitch`): accent track (NOT iOS green: the user disliked it on pure black), round white thumb with a
  squash at the ends; tap or DRAG the thumb (lands on the nearer side); touched, it lifts into the `Lift` lens.
  In drag callbacks read `positionChange()` BEFORE `consume()` (a consumed change reports zero movement). `SegmentedControl`: grey thumb that lifts into a lens on touch
  and can be slid to another option; release picks. Same drawn-at-rest / lens-while-held rule. Custom gestures inside
  scrolling pages use `awaitHorizontalTouchSlopOrCancellation` + `horizontalDrag` (consuming), and treat a null
  slop result as a tap only if the pointer lifted unconsumed: a swipe starting on the segmented control once
  switched the app to Light.
- Header capsules and the touchpad button: `glassChrome(..., interactive = true)` flexes the glass itself on press
  (grows past its bounds, zooms, brightens); glyphs inside use `LocalOnChrome` so `BounceIconButton` doesn't add a
  second drop. Never scale a glyph inside a still ring.
- Press glass: `pressable(glass = true)` / `pressScale(1f, glass = true)` = the dip-and-bounce press plus a soft
  glow under the finger. NO lens on button presses: refraction over a button's own label read as fake/glitchy
  (user rejected it on quick actions, pills, touchpad keys). With `pressScale(1f, glass = true)` it still dips (0.9).
- `liquidLens` is a Modifier.Node that allocates its layers only on first visible frame: cheap on every control.
- Icon buttons: `BounceIconButton` (never Material `IconButton`); glass circles/capsules put `pressScale` BEFORE
  `glassChrome` so the glass itself squishes, not just the glyph.
- Feature colors: `SystemColor` (iOS system palette, light/dark tones, no indigo/purple). `IconBadge` is a filled
  squircle with a white glyph; give each feature a stable color (notifications RED, messages GREEN, clipboard ORANGE,
  files/connection BLUE, touchpad TEAL, find YELLOW, battery GREEN, destructive RED, info/neutral GRAY).
- Control Center tiles: colored glyph on `glassRaised`; toggles fill with their color (white glyph) while on.
- Root navigation: `TabsRoot` (app/ui/TabBar.kt), iOS 26 tab bar: Home / Clipboard / Settings in a glass pill, the
  selected tab on a plain neutral capsule (colour only in icon + label). The drop lifts on touch-DOWN, jumps to the
  touched tab, follows the finger (tick per tab), picks on release; it never lifts without a finger. `glide` ignores
  a repeat of the current target: re-gliding mid-flight swapped the leading/trailing springs and overshot one way only. Touchpad is a
  separate round glass button. Tabs stay composed (keep-alive) and are only placed when shown.
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
- Long or unbounded groups (folder listings) are one lazy item per row, each painting its slice with
  `Modifier.cardSegment(first, last)` (flat cards have no rim, so slices join seamlessly) and cancelling the list gap.
  Never draw one card behind the list from the list's own draw: the overscroll stretch moves the rows, not it.
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
  on a resting screen: anything under the glass chrome forces the blur to re-render every frame.
  Scrolling is GPU-bound: measure per-frame GPU time with scratchpad-style framestats parsing (GpuCompleted -
  IssueDrawCommandsStart), not just jank %, which swings 2x run to run. Budgets found: chrome glass renders at
  quarter resolution (`CHROME_RES`) with the edge clipped and rimmed at full res (was ~4 ms of 8.5); the canvas is a
  cached quarter-res bitmap (three full-screen gradients were ~1.3 ms). Settings scroll GPU ~4.8 ms/frame. The connection orb
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
