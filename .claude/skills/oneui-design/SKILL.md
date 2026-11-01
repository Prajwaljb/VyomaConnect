---
name: oneui-design
description: VyomaConnect's visual and motion language (One UI / ColorOS / iOS inspired) and the designsystem module's building blocks. Read before creating or changing any screen, component, widget, tile or notification visual.
---

# Vyoma design rules

**Never purple by default.** No generic AI gradients. The look is One UI / ColorOS / iOS 27: glass materials over a
soft accent-tinted canvas, colorful feature icons, a floating glass tab bar. **Performance is a requirement**: no
per-frame blur except on chrome that content scrolls under, and measure with `dumpsys gfxinfo` after visual changes.

## Glass (core/designsystem/Glass.kt)
- Wide windows (landscape, tablets): `CollapsingHeaderScaffold` centres a column of at most `MaxContentWidth` (560 dp).
- Dashboard actions (user-approved 2026-10-31): SIX one-shot actions as filled Shortcuts tiles (16 dp radius, 12 dp
  card pad, 8 gap; tile size from `BoxWithConstraints`) + Lock and Mute as two SEPARATE square switch tiles (clear
  glass with a coloured glyph when off, WHITE 0.96 with the deepened coloured glyph and dark label when on, + a 1 dp
  tone hairline in light mode; `ToggleTile`). Shape rule (judge, from the user's CC refs + HIG): circles only for
  icon-only controls; anything with words is a rounded square/capsule. Never make the switches circles. A 2-wide shared module was tried: the halves
  didn't read as two ("bad implementation"). An ON switch must never look like an action. Slots: Send files (live
  progress ring) · PC files|Files tab · Screen|Ping · Camera|Send clip / Find PC · Commands (chevron) · Lock · Mute.
  No Keyboard tile. Hues off the meters' scale: Ping CYAN, Camera ORANGE, Find PC TEAL. Mid-luminance tiles use
  `toned(lightness = 0.38)` (mixing black turned orange brown). 
- Long press = a `LiquidMenu` (designsystem/LiquidMenu.kt) that POURS OUT of the pressed element: it starts as the
  element's own rect and radius, width on `leading()`, height on `jelly()`, into a 240 dp menu above (or below) it,
  with a tilt-lit rim, inner light band and soft shadow; items fade in once formed; dismissing flows it back into the
  element. The element itself only dips (`liquidPress`): a lens over the button was rejected ("doesn't even match the
  shape"). Menus: Send files (files / photos / clipboard), Screen (watch / screenshot), Send clip (send / history),
  Commands (the commands themselves).
- Pages earn their scroll with real content, never empty space (user: "make that content, make it impressive").
  Home after the tiles: "Run on PC" command capsules and "Captures" (newest 5 Pictures/Vyoma thumbnails + "See all"
  to the Captures page; `CaptureStore` loads them when Home opens, and the strip holds its height while loading: loaded
  lazily, the list ended short and then jumped).
  Files: Send (three big tiles: Files / Photos / Clipboard), On the PC (Desktop, Documents, Downloads, Pictures, Music,
  Videos: clear 16 dp glass tiles with a BARE coloured glyph, no badge inside, + Browse everything), Captures, Transfers, Clipboard.
- (Old) Dashboard actions: Apple Shortcuts-widget tiles, 4 across in one card, concentric corners (18 dp tile + 10 dp pad), white
  ink (dark on yellow-bright tones; tones of luminance 0.2–0.5 darkened 22% for 4.5:1). Now (2026-11-01, user chose it over "card with more air"): tiles FLOAT on the canvas like Control Center, no
  enclosing card, 16 dp gaps (a card with 8 dp gaps stacked nine lit outlines: "congested"); toggles off get their own
  body (white 0.14 dark / 0.55 light). Home groups by spacing only, no labels: +8 dp above the volume card and above
  the tiles; Captures on Home is one compact row (newest thumb, count, chevron; Files keeps the strip). 16 dp radius
  tiles, labels 11 sp SemiBold (as the other small labels), 9 dp even
  side padding so "Commands" fits at full size (measure label heights: auto-shrink read as uneven). Corner capsules
  (volume output picker) sit an even 10 dp from top and side. Toggle tiles keep
  fill AND name when on ("Locked", "Muted"): the user wants the state in words (a judge suggested constant labels; rejected).
- Layers (iOS 26 rule, judge-reviewed 2026-10-31): full liquid glass (lens, band, rolled edge, full tilt sway) only on the
  floating layer (header, title bubble, tab bar, touchpad button) and controls while held. Content cards are a thinner
  pane: band 0.15 dark / 0.35 light, no rolled shade, highlights sway 40% (`SpecularLight.content`). Filled raised
  controls: band 0.18, lit rim 0.35, band ≤3 dp under 64 dp. List IconBadges: edge 0.35/0.08 (hero glyph keeps 0.75).
- Density: Clipboard (a pushed page now) puts history first (Send in the header, setup row at the end); Settings
  subtitles are one line; Appearance (accent + theme) and Effects (glow, lighter glass) are separate cards; commands
  are one "Commands" row in PC details opening the sheet; the fingerprint is one line until tapped.
- PC status card: battery as an 80 dp hero ring (7 dp stroke, the only exception to 6 dp) + CPU/Memory/GPU/Temp as a
  2×2 of `Meter`s (label, `Reading`, 6 dp capsule bar). Never 5 equal gauges in a row (user: "5 in a row isn't nice").
- Touch, everywhere: `pressable` defaults to `PressAuto` (every edge moves 3 dp in, whatever the size) and a finger glow
  (white in dark, navy shade in light) on anything that dips. Order: `pressable` BEFORE the shape/glass modifiers so the
  glass squishes. Dialog actions are `DialogButton` (never Material TextButton); switch rows toggle on the whole row with
  the switch's haptic; keys tick on touch-down. Every icon-only tappable has a TalkBack label.
- Like elements, like sizes: every header bubble (back, title, actions) is `CHROME_SIZE` 48 dp; header padding = the page
  gutter. NO scroll-edge fade or status-bar shade (user, 2026-10-31: "I really want that black header shade to go away": it hid the glass). Round controls are true circles (`circle()` in the touchpad).
- Portrait only (manifest), except the PC screen viewer (sensorLandscape): the user doesn't want landscape anywhere else.
- The hero title at rest is the LIST's copy (stretches on overscroll with the page); the travelling copy takes over the
  moment the page moves. Meter labels are one line ("RAM", not "Memory", which wrapped beside 14.6G).
- Every hero is 30% of the screen, Home included; Home's tiles pass under the tab bar at rest (the user wants the
  glass to show content). Toggle tiles are clear glass off and white on (see ToggleTile); 0.2–0.5 luminance tones darken. One filled accent (`Glass.accent`)
  for switches, slider fill, chips, keys, pills. Media tint is clamped deep (luminance ≤ 0.06) for 4.5:1 text.
- Data visuals share one system: 6 dp round-capped strokes on a `Glass.raised` track (gauges and the battery rings),
  numbers with a smaller muted unit (`Reading`), colour = load (green < 70% < yellow < 90% < red; temp 65/85 °C),
  never a fixed colour per gauge. The bolt is `Icons.Rounded.Bolt`, never an emoji.
- Filled accent controls (prominent pill, send circle, chosen chip, toggled key) use `Glass.accent` (a deep shade of
  the accent) with WHITE ink in both themes; dark ink on the accent read muddy. Rejected: round buttons (plastic balls), wide 2-across tiles (abs), app icons.
- Canvas: `Modifier.aurora()` (theme background + two cached accent glows centred at the top (user: centred, not a corner); dark: dimmer glows (0.16 vs 0.20), the rest true black; `AppCanvas.glow`, from Settings > Background glow, turns them off).
  Dark mode IS pure black (AMOLED); there's no separate grey dark theme or toggle. No decorative textures (stars were tried and rejected: too specific).
- Depth tokens: `glassDepth` (`GlassDepth` Dark (true black) / Light) (light cards are see-through, white 0.42, so the canvas tint shows; never back to near-solid white) drive every surface so the themes stay consistent.
- Cards: `Modifier.glassCard(shape, fill?)` = a translucent even sheet (white 9.5% dark / 72% light;
  the user found opaque #1C1C1E "too grey" once cards had lit edges), faint shadow in light mode. Edge: a faint tilt-lit SWEEP hairline (`specularRim`; the user found a
  linear-gradient rim misaligned) plus, in the dark, a soft glow on the lit side baked INTO the fill as one radial
  gradient (a separate glow pass over the card cost 0.4 ms GPU/frame; in-fill is ~free). Measured ≈ +0.1–0.2 ms.
  Card slices (`cardSegment`) pass `lit = false`. Refraction belongs only to things that float.
- Photos (Captures thumbnails) are plain rounded images with a still hairline, never glass (a tilt-lit rim per
  thumbnail redrew them all as the phone moved).
- Media card (`MediaCard`): iOS-widget compact (full-bleed art was "too big", a thumbnail in a plain card "too
  plain"): artwork fills the left 132 dp edge to edge, fading into the art's dark-muted colour; title/artist with
  Continue on phone as a 36 dp round button, thin scrubber, plain glyph controls on the right. Card = `glassCard`
  (tint fill + lit glow) with `specularEdge` drawn over everything. Art trimming/palette run off the main thread.
- Liquid drops (`LensPainter`: tab drop, slider/switch/segmented knobs) get the tilt-lit `specularRim` stroke like
  chrome; a clear drop over a plain track otherwise had no visible rim.
- Anything pressable on a card (buttons, chips, keys, tiles, PillButton): `Modifier.glassRaised(shape, color?)`, a plain
  even fill plus a 0.9 dp specular hairline that follows the phone's tilt (a FIXED painted rim read as fake; the user
  asked for it back once it moved like iOS 26); pass `color` for accent/tinted or for chips over artwork (dark translucent).
- Light: `SpecularLight.angle` (Light.kt) is where the light is on screen (270° = straight up, centred, at rest: the user's call; tilt sways it up to ±55°, soft limit, gain 2.2: ~10° of turn reaches straight up; the user found ±35° could never bring it to the centre), driven by the
  gravity sensor only while the UI is started (`TrackSpecularLight` in `VyomaTheme`), smoothed, 1.5° dead band. Every
  specular edge uses it: `specularRim(...)` (chrome stroke, raised controls) and the AGSL `light` uniform (lens rims).
- Rims follow Apple's iOS 26 glass (user's reference shots): a broad highlight facing the light plus one ~80% as bright
  opposite (`specularRim(opposite = 0.8f)`), and `InnerGlow` (one per element), a soft band of light a few dp deep just inside the
  edge (cards 8 dp at `GlassDepth.band`, chrome ≤4 dp at 52% white in dark, raised ≤4 dp; t³ falloff, k^1.6 towards
  the lit sides; the user tuned these for contrast, not spread). Hairline-only rims read as dull grey slabs on true
  black; the band is what makes it glass. Cards and chrome also pass `shade`/`depth` (cards 16 dp, chrome ≤10 dp; kept
  subtle: the user found 0.6 shade "too 3dish"). Raised controls don't: small grey circles with it read as plastic
  balls. The user wants the glass clear: dark fills ~0.05 white, light cards 0.32 white, chrome tint 0.42/0.38: the edge is shaded as a thick drop rolling over (brighter sloping towards the light, `GlassDepth.shade`
  sloping away, a shallow counter-curve past the roll), so flat sides read as 3D like the WWDC25 glass reference. Lite glass keeps the band. It must fade with
  no visible steps (stacked strokes showed rings): an AGSL distance-to-rounded-rect pass, many faint strokes below API 33.
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
  chrome element uses the tab bar's values: `blur = 6.dp, bevel = 16.dp` (8 until the user asked for slightly less blur, 2026-10-31). The bar must not be inside
  the source. Compose/draw chrome only while visible.
- Header (`CollapsingHeaderScaffold`): no slab, no fade (content slides under clear glass). Back in a 48 dp liquid circle; actions share
  one capsule. The title is ONE travelling Text (the hero copy is an invisible placeholder): it scrolls 1:1 with the
  page (no hero parallax, else it slid over the hero glyph), pins in the bar and shrinks to titleMedium; at collapse
  docks (within 6 dp of its bar position) a glass bubble forms around it (height leads, width wobbles out with `jelly()`).
  The bubble belongs to the title: the moment the title leaves, it rides down with it and is gone within 16 dp, then
  POPS (swells 18%, 140 ms). A fixed 220 ms fade at a collapse threshold lagged fast flings (empty bubble behind the
  text). Forming and bursting each play `Haptic.Bubble`; a list flung into either end ticks (`Detent`). The hero fades by 75%.
  Under the status bar: `scrollEdge(backdrop)` (iOS 26 soft edge), only while scrolled: blurs what passes under
  (progressive: a 14 dp blur / 0.4 tint reaching 48 dp below the bar plus a 32 dp blur / 0.35 tint over the top 18 dp;
  lite 0.9 tint), masked to fade out (the user loved it and
  asked for it longer and stronger than the first 10 dp blur / 12 dp reach). The user found raw
  cards under the clock "not nice"; a dark shade stays rejected.
  The hero title's centre sits at a fixed 63% of the hero (56% put the PC glyph up against the status bar) on every page (custom Layout, displayMedium everywhere):
  the device glyph (64 dp) hangs 14 dp above it, the subtitle 6 dp below. Centring the whole block moved the title
  by up to 46 dp between pages.
  A trailing "collapse-room" item lets any page that scrolls at all collapse fully (no half-faded stuck title).
  A page whose content ends ≥12 dp above the tab/nav bar doesn't scroll at all (`userScrollEnabled = false`). The
  check re-runs on every layout and answers "overflows" whenever the last real item is off screen: once it kept a
  stale "fits" from a short first load (folder loading, Home before the PC) and the grown page never scrolled.
  A page whose content all shows above the tab bar doesn't scroll at all (`userScrollEnabled = false`): scrolling
  only to collapse the title read as broken. The hero is the same height on EVERY page (`heroFraction` 0.30, One UI);
  Home once took less and the user called it inconsistent.
  A scroll that stops mid-collapse snaps open/closed; the snap runs in its own child job (run inline, a finger
  interrupting it threw a cancellation that killed the watcher, and nothing snapped again).
- Long-press menus: `MenuHost` (wraps the app in MainActivity) + `LocalMenu.current.show(boundsInRoot, items, title)`,
  triggered from `liquidPress(onLongClick)`. Drawn IN the app's window (a Popup is a separate window: it can't see the
  page, so it was an opaque slab the user called broken): while open the app records into a backdrop and the menu is
  `glassChrome(blur = 13.dp, bevel = 16.dp)` (blur above the tab bar's 6 so its text reads over a busy page), 24 dp
  corners, 35% black scrim, tap outside or Back closes. It pours out of the tile's own rect (width `leading()`,
  height `jelly()`), items fade in past ~55% formed, and flows back in on close. Placement: above the tile if it fits,
  else below, else the roomier side, always inside the system bars. The lens takes the shape's corner radius from its
  outline (it used to assume a capsule, which drew a second square outline around a rounded-rect menu).
- Dialogs: `GlassDialog` (same API as AlertDialog): opaque `surfaceContainerHigh` slab with the card rim. Never
  Material `AlertDialog`.
- Value bars (Apple's, `LiquidSlider`): with `knob` (default 24 dp) the iOS 26 slider: slim track, wide white
  capsule knob; `knob = null` is the Apple Music scrubber: bare track that thickens under the finger. Relative drag
  after horizontal slop. ONE knob throughout (continuity): at rest a drawn `frostedKnob` (no layers: a resting lens
  cost scroll frames); held, `liquidLens(style = LensStyle.Lift, frost = { 1 - swell })` of the same rect grows ~45%
  and clears; an inward rim mirrored the track into stripes. Volume (`Dashboard.VolumeBar`) is an iOS card: name +
  % over the knobbed slider between quiet/loud speakers. The media `Scrubber` drives one `Animatable` progress (small
  drifts snap, jumps glide) and holds our seek target until MPRIS agrees (browsers report stale/zero after a seek).
  Level rows in a card (`PcControls.LevelRow`: PC brightness, per-app volume): badge + name + % on one line, the
  slider under them inset 50 dp so it lines up with the name; a mute toggle is its own speaker button after the %
  (never the badge). Any slider driving a remote value owns it while touched and for 700 ms after (`ECHO_QUIET_MS`),
  then settles on what the remote reports. Power actions are plain rows; each opens a GlassDialog (destructive
  red except Sleep).
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
- Root navigation: `TabsRoot` (app/ui/TabBar.kt), iOS 26 tab bar: Home / Files / Settings in a glass pill (user
  approved 2026-10-31: clipboard sync is ambient, so it's a section of Files + a pushed full-history page), the
  selected tab on a plain neutral capsule (colour only in icon + label). The drop lifts on touch-DOWN, jumps to the
  touched tab, follows the finger (tick per tab), picks on release; it never lifts without a finger. `glide` ignores
  a repeat of the current target: re-gliding mid-flight swapped the leading/trailing springs and overshot one way only. Touchpad is a
  separate round glass button. Tabs stay composed (keep-alive) and are only placed when shown.
  Lists get `LocalBottomBarInset` padding automatically via the scaffold.

## Theme (core/designsystem)
- Type: bundled Inter variable (`res/font/inter.ttf`, OFL in `core/designsystem/licenses`): display optical size
  for title/headline/display styles, text cut below; tight negative tracking on large sizes; `tnum` for live numbers.
- `VyomaTheme(ThemeSettings)`: palettes `DYNAMIC` (wallpaper accent, neutral surfaces), `OCEAN`, `GRAPHITE`, `EMBER`,
  `MINT`, `SAND`; DarkMode SYSTEM/LIGHT/DARK (dark = true black).
- Lighter glass: `GlassQuality.lite` (set by the app from Settings > Lighter glass, or by `FrameBudget` when the
  median scrolling frame takes ≥2.2x the refresh interval twice running). `glassChrome` then draws a 92% frosted
  fill + edge instead of blur/refraction (read in the draw phase, so it switches live); gauges jump, no glide.
  Every new live-glass element must honour it.
- Canvas `aurora()`, cards `glassCard`, raised chips `glassRaised`, dialogs `GlassDialog`.
- Status colors via `Status.connected/pending/offline(dark)`, the same in every palette.
- Contrast (user asked for more, 2026-10-31): text #F7F8FA / #0B0C0E, secondary #AEB3BB / #50565F, light canvas
  #EDEEF2 under white cards. Wallpaper (dynamic) accents get `saturate(1.35)` at the same lightness (Material You
  tones are muted). For a white glyph on the accent use a deep shade, `accent.toned(1.2f, 0.5..0.64f)` (the hero
  `DeviceGlyph`), never the pastel dark-theme accent itself.
- Shapes: cards `large` (26 dp), hero cards `extraLarge` (32 dp), chips/icons `medium` (20 dp).

## Layout
- Every screen must hold up at large text and display sizes (OnePlus defaults are larger than Samsung's). Test with
  the emulators (`vy37` API 37, `vy31` API 31 = minSdk, AOSP images) at 420/1.0, 480/1.15, 560/1.3 and 720x1600@320/1.3
  (`wm density`, `settings put system font_scale`). Rules: fixed copy wraps (no `maxLines = 1` on descriptions; only
  dynamic names/history ellipsize); sets of labels share one size via `fitTogether` (tiles, swatches) instead of each
  auto-shrinking alone; side-by-side name + reading pairs stack together when the widest doesn't fit (PC meters);
  fixed-width art yields (media art <= 38% of the card); tiny tile grids tighten gaps/padding before shrinking text.
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
- Where things live (IA): Home is the PC's control surface, stacked (the Control Center deck with a vertical volume
  capsule was built and SCRAPPED by the user, 2026-10-31): the PC's NAME is the hero (no glyph; subtitle = transport,
  or "Away · seen 12 min. ago"), ONE status card (battery ring + CPU/RAM/GPU/Temp meters with the companion), the
  media card (only while something plays), the volume card (output name as a capsule that opens the output list;
  no chip rows), the tile card, then "Run on PC" command capsules and "Captures". A paired PC that's AWAY keeps Home:
  an AwayCard (Try again + Use USB), dimmed cached status, no media/volume, tiles at 40% and untappable. Files tab:
  Send tiles, On the PC folders, Captures, Transfers, latest clips + "All clipboard history". PC details (header
  "i") leads with Connection (Link + Ping, Find this PC + Ring, battery), then one Commands row, then Security.
- Outcomes show in the status bubble (`LocalStatus.current.show(text, icon?, failure?, haptic?)`, designsystem/Status.kt):
  an opaque glass capsule that drops from the top, swells on jelly, holds 1.6 s (2.6 s for failures). Never Toast
  inside MainActivity (separate small activities and the VM still toast: they have no bubble host). Pass
  `haptic = false` when the caller already played one.
- First run: the radar sweeps 3 times and rests (tap it or "Search again" for more), three numbered steps, address
  and USB folded behind "Other ways". After pairing, a one-time "Get more from your PC" card offers the companion.
  The pairing code fills in a character at a time (40 ms apart, key tick each).
- Horizontal chip rows never cut hard: `Modifier.fadingHorizontalScroll(state, bleed)` (designsystem/Scroll.kt) fades
  each edge only while there's more that way; rows in the page gutter or a card bleed to its edge (`bleed` = the
  padding, content padded back). Capped vertical lists in dialogs use `fadingVerticalScroll`.
- One row anatomy: every tappable row is a `ListRow` (badge, 20 dp inset; `horizontalPadding = 0.dp` inside dialogs)
  with a trailing `RowChevron(state?)` ("On"/"Off") or a pill. Commands and fingerprints: `subtitleStyle` monospace,
  commands one line. Unbounded groups (clipboard history, files) are `cardSegment` slices + `joinNext()`.
- Each fact once per screen (the details hero says how it's connected; the battery row owns the charge). One name per
  feature everywhere ("Sync when closed"). No user-facing "ADB".
- One clear primary action per card. Touch targets ≥ 48 dp; keep the bottom 70 px clear of the floating tab bar
  (`LocalBottomBarInset`).
- Controls: `PillButton` (prominent = accent fill, else tinted), `SegmentedControl` for 2–4 exclusive choices,
  `VyomaSwitch` for on/off. No Material `Button`/`FilledTonalButton`/`Switch`/`SegmentedButton` outside dialogs.

## Efficiency
- Measure CPU with the app COMPILED: `adb shell cmd package compile -m speed-profile -f dev.vyoma.connect` after a
  launch (sideloads sit at `status=verify`, i.e. interpreted, ~2x the CPU). `app/src/main/baseline-prof.txt` compiles
  all of Vyoma's code at install. Scratchpad `cpu.sh N` = per-thread ticks over N s; simpleperf works (`--app`).
- Never repost a notification or session state per packet: PC packets restamp `positionAt` on every update, so key
  on what's shown (PcMediaSession) and identity-reuse unchanged bitmaps (MediaHub `sameAs`).

## Motion
- Springs only (`VyomaMotion`): `spatial()` movement (slight overshoot), `effects()` alpha/color (never bounces),
  `release()` anything let go of (iOS wobble), `page()` navigation (no overshoot), `leading()/trailing()` pairs for
  liquid shapes (lens, switch thumb): the edge moving toward the target leads, the other trails, so it stretches.
- Respond on touch-down, not on tap-up: `pressable` / `pressScale` dip immediately (Modifier.Node, layer-only) and
  bounce on release like water: width settles on `release()`, height on the softer `jelly()`, so it wobbles out of
  step instead of just scaling. If a scroll steals the gesture they ease back without the bounce.
- A finger that slides off a pressed element lets it go (12 dp margin) and dips it again if it comes back, like iOS;
  any release without a tap (slid off, scroll took it) still flows home with a soft wobble from wherever the dip
  was (a flat ease read as the press stalling halfway).
- Long-press menu motion: opens width-first (spring 0.8/700) with the height pouring after (0.6/340); items fade in
  past 60% formed. Closing: items (110 ms) and the glass fade together while the drop drains toward the element,
  height first (critically damped 340) and width after (220), no bounce; the glass is fully transparent by 35%
  drained and the menu ends there (draining all the way, it sat over the button as an opaque blurred blob and then
  vanished: the user called it glitchy). While it closes the overlay swallows touches, so the page can't scroll until the drop is home (user request). The pressed element
  stays undimmed (scrim has a hole). Tiles MORPH instead (`MenuMorph(owner, color, face)`): the tile hides
  (`covers(owner)`), the drop starts as an exact copy (its colour + `TileFace`, held in place) whose colour clears
  into glass over the first half of forming, corners 16 -> 24 dp; closing drains fully back, refills the colour,
  then hands over. The user asked for "the tile's glass itself expanding into the menu". It grows IN PLACE from the
  element's own corner (down from its top edge, or up from its bottom when there's no room; aligned to its nearer
  side), never popping out above/below it. With `icon`/`iconAt`/`ink`/`label`, the tile shrinks to its icon: the face
  (minus the icon) fades by 25% formed and the icon glides onto the menu item that already uses the same icon (never shown twice), else into a 44 dp header slot beside the label, crossfading
  from tile ink to onSurface. The PC output capsule morphs the same way (face only, no icon). The overlay gives the face `LocalContentColor = onSurface` (plain text there drew black, then flipped white at handover). Media player picker
  still pours from its button, current item checked; never a dialog. Morph open/close measured ~8% janky (lens rebuilt per frame). Tiles with a hold menu show a quiet
  "•••" (MoreHoriz, 60% ink) in the chevron's corner; one mark per tile.
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

- Measured 2026-10-31 (S24 FE, demo compiled): scrolling is render-thread bound (~3.5 ms issuing, a third of it
  the chrome glass layers' blur + lens passes, 18% pipeline switches); UI thread ~0.4 ms. Rules found:
  never write Compose state from `onGloballyPositioned` on things inside a scrolling list (a tile's bounds state
  woke the main looper every frame for every tile: keep `LayoutCoordinates` in a plain holder via `onPlaced` and
  read bounds on demand); AGSL `RuntimeShader`s are compiled once and shared (each draw/RenderEffect copies the
  uniforms); the aurora bitmap is cached app-wide (`AuroraCache`), not painted per page. A pushed page composes in
  one ~25 ms frame: navigation parks it at progress 0 (`seekTo(0f, target)`), waits two frames, then slides, so the
  spring doesn't jump. Tried and dropped: drawing InnerGlow as edge-only strips (no GPU gain, more draw calls).

## Haptics (`Haptic` in Motion.kt; always through `HapticFeedback.perform`)
Compositions of vibrator primitives where the motor has them; a missing primitive is swapped for its nearest
(low tick -> tick -> click, thud -> click; a rise is dropped); with nothing left, the phone's predefined effect
(`Haptic.predefined`: click/tick/heavy click/double click), which always vibrates and is the maker's own tuned
haptic (O-Haptics etc). Never fall back to view constants: OEMs map some to nothing (silent on a OnePlus).
Pixel / O-Haptics style: each `Haptic` is a `VibrationEffect.Composition` of primitives (CLICK, TICK, LOW_TICK, THUD,
QUICK_RISE; scaled, some paired: ToggleOn rises tick→click, ToggleOff falls click→low tick, Success two rising clicks),
played by `Haptics` (attached in `VyomaTheme`) with USAGE_TOUCH so the system touch setting applies; phones without
those primitives fall back to the view's stock constant. This S24 FE's THUD lasts 300 ms: use it only for Failure.
Picking one of several (chips, players) is `Select`.
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
- Menus (`MenuHost`): the page and the overlay each have their own `graphicsLayer()`. Sharing the root layer, every
  menu frame re-ran the backdrop recording and re-rendered the whole page. While a menu is up the backdrop layer is
  `Offscreen` (a cached texture drawn twice: on screen and into the glass); recording stays on always (toggling it
  re-recorded the page, ~7 ms). Measured: pour frames' GPU median 8.4 -> ~5 ms. Anything that changes the page under a
  menu re-renders the whole texture (~10-15 ms), so hand-paced glides (`rememberGlide`) jump while `menus.isOpen`.
- Tried and dropped: quarter-res `scrollEdge` blur (no measurable gain; Skia already downsamples big blurs).
  Compare builds A/B alternately (`ab.sh`-style), never one after another: thermal drift is as big as the effect.
- Every chrome element replays the page behind it, so a page-level offscreen buffer is multiplied by the number of
  glass elements. Never fade pages/large content with a default layer alpha: use
  `compositingStrategy = CompositingStrategy.ModulateAlpha`, or draw a shade (push/pop dims the page below this way).
- Idle must be idle: periodic data (stats every 2 s) snaps small changes and glides big ones at ~30 fps by hand
  (`withFrameMillis` + `delay(33)`; `preferredFrameRate` is not honoured here); clocks tick on the second.
  Measure: `dumpsys gfxinfo` frames over 6 s at rest, and `framestats` (ui vs issue vs gpu) for transitions.
