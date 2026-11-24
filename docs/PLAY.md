# Google Play

The Play build is the `play` build type: `./gradlew :app:bundlePlay` makes
`app/build/outputs/bundle/play/app-play.aab`. It is the release build (same id `dev.vyoma.connect`, same
key, R8) with `BuildConfig.PLAY` true and `app/src/play/AndroidManifest.xml` for taking out what Play
refuses (`tools:node="remove"`). Today it takes nothing out: every permission goes to review first.
Checked on the first AAB (2026-11-20): signed with the release key, the four native libraries (AndroidX
and CameraX) aligned to 16 KB pages, no dependency list for Google, 33 permissions, versionCode 13.

## The owner's decisions (before the first upload)

- **Account.** A personal account made after 13 Nov 2023 must run a closed test with at least 12
  testers for 14 days in a row before it can publish; an organisation account (D-U-N-S number) needn't.
- **App signing.** Play App Signing with *this* key uploaded (Console: "Use a different key" → upload
  with Google's PEPK tool) keeps one signature on Play, GitHub, F-Droid and IzzyOnDroid: people move
  between them without reinstalling, and the PC's USB cable allow-list (it goes by the APK's signing
  key) keeps working. Letting Google make the key instead means the Play app can't update over a GitHub
  install, and the other way round. Either choice is final.
- **Title.** "Vyoma" alone may be taken on Play; up to 30 characters, e.g. "Vyoma: phone and PC as one".

## Listing (from fastlane/metadata/android/en-US)

- Title, short description (75 of 80 characters) and full description (2,326 of 4,000) fit.
- Icon 512×512: fits.
- **Screenshots: need new ones.** Play wants 2 to 8, each side 320 to 3,840 px, the long side at most
  twice the short; the three there are 540×1116 (2.07:1). Take them from the demo build at 1080×2160 or
  crop the status bar off.
- **Feature graphic: missing.** 1024×500, needed for the listing.
- Privacy policy: https://prajwaljb.github.io/VyomaConnect/privacy.html (`site/privacy.html`).
- Category: Tools. Contact: an email address is required on the listing (the owner's choice).

## Declarations

Each restricted permission gets a Console form, most with a short video of the feature in use. Texts to
start from:

**SMS and call log** (READ_SMS, SEND_SMS, READ_CALL_LOG). Exception: "Cross-device synchronization or
transfer of SMS or calls". *Vyoma links the phone to the user's own computers (KDE Connect protocol).
It shows the phone's texts on the computer and sends replies written there through the phone, and shows
incoming and missed calls on the computer. Messages go only to computers the user paired, encrypted
over the local network; nothing goes to a server.* Video: a text arriving and answered on the PC.

**All files access** (MANAGE_EXTERNAL_STORAGE). *The user's paired computer browses the phone's storage
(SFTP over the local network) to copy files both ways, like a file manager on the computer.* Play allows
it for file managers, backup, document management and device-to-device migration, so this may be
refused. If so: remove it in `src/play` and offer folder grants (the storage access framework) for the
PC's browsing in the Play build (`BuildConfig.PLAY`).

**Accessibility service** ("Vyoma PC control", `MirrorAccessibility`; not an accessibility tool). *While
the phone's screen is shown in a window on the user's computer, it performs the taps, swipes and typing
from that window. It reads only the focused text field, to type into it, and the selection on Ctrl+C,
which goes to the user's own computer.* The in-app disclosure is the dialog on the mirror screen before
Accessibility settings open (MirrorActivity, "Control from the PC?"): show it in the video.

**Foreground service types**, each with a video:
- connectedDevice: the link to the paired computer (always while linked).
- mediaProjection: the phone's screen in a window on the computer.
- camera and microphone: the phone as the computer's webcam and microphone; the microphone also
  carries the phone's own sound while its screen is shown on the computer.

**Full-screen intent** (USE_FULL_SCREEN_INTENT). Used only for Find my phone's ringing screen on a
locked phone. Play grants it by default to calling and alarm apps only, so answer that Vyoma is neither:
the permission stays off unless the user allows it, and the ringing still sounds with a heads-up
notification.

**Battery optimisation** (REQUEST_IGNORE_BATTERY_OPTIMIZATIONS). *A companion app that keeps a
connection to the user's computer, so calls, texts and notifications reach it while the phone sleeps.*

**Not restricted but worth knowing:** READ_LOGS and SYSTEM_ALERT_WINDOW (background clipboard, granted
by the user's PC over adb, never at runtime); SCHEDULE_EXACT_ALARM (time-based automation rules; the
user grants it, no declaration).

## Data safety

Nothing reaches the developer: no server, account, analytics or crash reporting. Data goes only to
computers the user paired, end to end encrypted (TLS with keys the user confirmed). Play counts data
"collected" when it leaves the device; data sent to the user's own devices, end to end encrypted, is a
judgement call in the form, so read Play's current wording before answering. The honest summary for the
form: no data shared with third parties; encrypted in transit; the user can delete it (uninstall, or
unpair a PC); the one third-party request is album art loaded from the address the PC's player names.

## Still to do

- New screenshots and the feature graphic.
- A video per declaration.
- The closed test, if the account needs one: the AAB to an internal test track first, then closed.
