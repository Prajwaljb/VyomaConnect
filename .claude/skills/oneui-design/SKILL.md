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
  inside the source. Compose chrome only while visible (the collapsed header does this).
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
- Group related rows in `SectionCard` with `ListRow`s (`iconTint = SystemColor.X.color` → colored squircle `IconBadge`,
  dividers inset past the icon).
- Section titles use `SectionLabel` (small, muted, above the card).

## Motion
- Use `VyomaMotion.spatial()` for movement/scale (slight overshoot) and `effects()` for alpha/color (no bounce).
- Everything tappable uses `Modifier.pressable` (scale squish plus haptic tick). Don't use the ripple-only `clickable`.
- Navigation: push = slide-in third + fade; back = predictive-back shrink + rounded corners (MainActivity).
- List changes: `Modifier.animateItem()`; size changes: `animateContentSize()`.
- Drive scroll-linked effects from `graphicsLayer {}` lambdas reading state, never via recomposition.
- Live status = `ConnectionOrb` (breathes when connected, fast pulse while pairing).

## Widgets (Glance, Phase 1)
Same palette tokens; system widget corner radius; never show a spinner forever. Render cached state and an
"offline" look when the PC is away.

## Copy
Short, friendly, specific ("Connected via Wi-Fi", "Make sure this code matches on both screens"). No exclamation spam.
