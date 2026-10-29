---
name: oneui-design
description: VyomaConnect's visual and motion language (One UI / ColorOS / iOS inspired) and the designsystem module's building blocks. Read before creating or changing any screen, component, widget, tile or notification visual.
---

# Vyoma design rules

**Never purple by default.** No generic AI gradients. Surfaces are neutral, and the accent is the only color.

## Theme (core/designsystem)
- `VyomaTheme(ThemeSettings)`: palettes `DYNAMIC` (wallpaper accent, neutral surfaces), `OCEAN`, `GRAPHITE`, `EMBER`,
  `MINT`, `SAND`; DarkMode SYSTEM/LIGHT/DARK; `amoled` = true black.
- Canvas `background`, cards `surfaceContainerLowest`, raised chips `surfaceContainerHigh`.
- Status colors via `Status.connected/pending/offline(dark)`, the same in every palette.
- Shapes: cards `large` (26 dp), hero cards `extraLarge` (32 dp), chips/icons `medium` (20 dp).

## Layout
- Every top-level screen uses `CollapsingHeaderScaffold`: a tall hero title (~30% of screen height) that keeps content in
  thumb reach, collapsing into a compact bar. 16 dp side gutters, 12 dp between cards.
- Group related rows in `SectionCard` with `ListRow`s (icon in a tinted circle `IconBadge`, dividers inset past the icon).
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
