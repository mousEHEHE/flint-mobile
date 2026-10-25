# flint-mobile

> [!WARNING]
> An experiment: can flint run on a phone with the same Rust and gpui as the desktop app?

flint for iOS and Android. The UI is [gpui](https://www.gpui.rs/), on the
platform layer of [gpui-mobile](https://github.com/longbridge/gpui-mobile) by
Longbridge. SSH, the terminal emulator and the vault come from flint-core, the
same code the desktop app runs.

## Folders

```
~/dev/flint/
  flint-core/    SSH, terminal emulator, vault, sync. No UI.
  flint-gpui/    the desktop app
  flint-mobile/  this app
  gpui-mobile/   a clone of longbridge/gpui-mobile with fixes not merged yet
```

## What there is

- **Your flint account.** Sign in with GitHub, then the master password once.
  The phone keeps the vault key in the keychain, so the app opens straight to
  the hosts. Only accounts made in flint on a computer: the phone does not
  create one.
- **Hosts** from the vault, by folder, with team hosts in their own sections,
  each with the mark of the system it runs, as on the computer. A row of
  folders and `#tags` over the list narrows it to one. A host's card lists its
  address, login, bastion, system, folder and tags. Search at the bottom,
  where the thumb is. Add, edit and delete hosts; the change goes to the
  account at once. Folders, tags, bastions and forwards are kept as they are
  and edited on the computer.
- **Sessions** keep running while the hosts list is in front, each as a card
  with its last two lines, and a switcher goes between them; the one seen
  last comes first. Password and key logins, bastions and proxies from the
  vault, a key passphrase asked once per session, the host key saved to the
  vault on first trust, 2FA prompts.
- **Find in output** from the session's menu: every match in the scrollback
  is marked, the arrows step through them and scroll each into view.
- **Files** (SFTP) over the session's own connection: the folder button in
  the session's bar, or Files on a host's card. Folders, then files, each with
  an icon for what it is; hidden files on request. Look at text and images on
  the phone; Share or save sends a file to the share sheet on iOS (Save to
  Files, AirDrop, other apps). Upload from the phone with +: an upload never
  replaces a file and takes `name (2).ext` instead. New folder, rename, move
  to another folder, delete, permissions (read, write and run for the owner,
  the group and others), copy the path, open the folder in the terminal.
  Progress and cancel for every transfer. The last folder is remembered per
  host, as on the computer.
- **Editing** a text file of up to 1 MB: Edit on its sheet or in its preview,
  then Save writes it back. If the file changed on the server since it was
  opened, the phone asks before it replaces it; leaving with changes asks too.
- **Snippets** from the vault, run in the session with one tap.
- **Settings**: light, dark or the phone's own appearance; the terminal's
  font size and colours, with a preview.
- Every field gets the keyboard it needs: passwords a secure Latin one, the
  address one with `@` and `.`, the port digits.
- **Quick connect** with host, user and password, without saving.
- The desktop's terminal: the same emulator and the same drawing code.
- A keys bar over the keyboard: snippets, esc, ctrl, alt, tab, arrows,
  `| / - ~`, home, end, page up and down, paste. Ctrl and Alt: one tap for the
  next key, two quick taps to lock.
- Tap: the keyboard. Pan: scrollback. Pinch: font size. Long press: select a
  word, then Copy.
- Back from the background: a dead connection is noticed in a few seconds
  and connects again; the vault syncs again.

Not yet: port forwarding, making keys, team management, the Android Keystore
(Android keeps the vault key in a file in the app's own folder).

## Build and run

You need [rustup](https://rustup.rs/). The toolchain and the phone targets
install themselves on the first build.

**iOS** needs Xcode and XcodeGen (`brew install xcodegen`).

```sh
./build.sh ios-sim      # the iOS Simulator
./build.sh ios-device   # the iPhone connected to the Mac
./build.sh ios          # opens Xcode: pick your iPhone and press Run
```

The first time on an iPhone: sign in to Xcode with your Apple ID (Xcode >
Settings > Accounts), connect the phone with a cable, unlock it and tap Trust,
and turn on Developer Mode (Settings > Privacy & Security, the phone
restarts). With a free Apple ID, also trust yourself on the phone after the
first install (Settings > General > VPN & Device Management), and install
again every 7 days, when the free profile ends.

**Android** needs the Android SDK with the NDK (Android Studio installs both),
`cargo install cargo-ndk`, and a phone with USB debugging on.

```sh
./build.sh android            # debug build
./build.sh android --release  # fast Rust code, still a debug-signed APK
```

Or without the Android SDK: GitHub builds the APK with
`.github/workflows/android.yml`, with flint-core and the `flint` branch of
gpui-mobile's fork next to it.

```sh
gh workflow run android.yml
gh run watch
gh run download -n android-arm64   # Flint-<version>-android-arm64.apk
adb install -r Flint-*.apk          # or open the APK on the phone
```

The workflow signs the APK with the debug key in the `ANDROID_DEBUG_KEYSTORE`
secret, so each build installs over the last one and keeps its data. The key
is made once. The same file in `~/.android/debug.keystore` lets the APKs from
`./build.sh android` install over the ones from GitHub too.

```sh
keytool -genkeypair -keystore debug.keystore -alias androiddebugkey \
    -storepass android -keypass android -keyalg RSA -validity 10000 \
    -dname "CN=Android Debug,O=Android,C=US"
base64 -i debug.keystore | gh secret set ANDROID_DEBUG_KEYSTORE
```

Logs: the terminal on iOS (also in `ios/build/app.log` for the simulator and
`ios/build/device.log` for the iPhone) or the Xcode console with `./build.sh ios`;
`adb logcat -s flint:V gpui-mobile:V` on Android.

**On the computer**, for working on a screen without a phone:

```sh
cargo run --example preview
```

An iPhone-sized window with the same UI. No on-screen keyboard and no phone
APIs, so check typing and the background on a phone. The preview keeps its
data in `~/.flint-mobile-preview`, never in the desktop app's folder. For a
vault of sample hosts without signing in:

```sh
cargo run --example seed_preview   # replaces the preview's vault
```

The first build compiles gpui and takes several minutes.

## What to check

- Sign in with GitHub: a sheet over the app, which closes by itself. Then the
  master password; the hosts come in a moment later. Quit and start the app:
  it opens without asking.
- Add a host on the phone and see it on the computer, and the other way round.
- Connect to a server, run `htop` and `vim`.
- Files: open a host's files, look at a text file and a photo. Share or save a
  file: the share sheet comes up, Save to Files keeps it. Upload a photo from
  the phone with +, then the same photo again: it lands as `name (2)`. Rename,
  make a folder, delete. On Android, Share or save says it is not ready:
  gpui-mobile has no save dialog or file share there yet.
- Edit a config file, save it, and `cat` it in the terminal. Change it in the
  terminal while it is open on the phone, then save on the phone: it asks
  first. Move a file into another folder; give a script the run permission.
- Find in output: search for something `ls` printed, step with the arrows.
- On the hosts screen, pick a folder and a tag above the list.
- Type on the on-screen keyboard, in the Russian layout too. No autocorrect,
  no capital letter at the start, `"` and `--` stay as typed.
- Esc, Ctrl+C, Ctrl+R, Tab, the arrows from the keys bar.
- Scroll back, select a word, copy, paste.
- Turn the phone: the terminal gets new columns and rows.
- Go to another app for a minute and come back: the shell is there, or it
  comes back on its own.
- An idle session does not warm the phone or drain the battery.

## Changes in ../gpui-mobile

Branch `flint` of the clone, also on the fork (`mousEHEHE/gpui-mobile`), where
the Android workflow gets it. On top of the two open PRs below, one commit for
each of these:

1. **iOS: strings without NUL.** `stringWithUTF8String:` got a Rust `&str`,
   which has no NUL at the end, so it read past the string. Copying to the
   clipboard, opening a link, alerts (title, message and buttons) and
   `packages::share::share_text` could pick up garbage after the text. They now
   use `util::nsstring`. For a PR.
2. **Android: Backspace for a view without its own text.** The keyboard
   deletes through the focused input's text, and a terminal has none, so
   Backspace did nothing. Now such a view gets Backspace and Delete key
   presses. For a PR.
3. **iOS: building without the video player and camera.** `platform_view.rs`
   used both packages without their feature flags, so the crate did not
   build for iOS with only the packages flint needs. For a PR.
4. **iOS: no smart punctuation.** Curly quotes and long dashes break
   commands. This turns them off for every input; for a PR it needs an
   option per input.
5. **iOS: an asset source.** `ios::ffi::set_assets`, the iOS side of
   Android's `start_with_assets`. Without it icons and other SVGs come back
   empty on iOS. For a PR.
6. **A password keyboard.** `KeyboardType::Password`: secure entry and a Latin
   keyboard on iOS, `TYPE_TEXT_VARIATION_PASSWORD` on Android. iOS also
   reloads the keyboard when the type changes while it is up, so moving from
   one field to the next shows the right one. The example's Form screen has a
   Password field to try it. For a PR.
7. **iOS: typed text stays out of the log.** Every keystroke was logged at
   `info` with its text, passwords included. Now only its length, at `trace`.
   For a PR, first of these.

Not changed, but worth a PR: `packages/file_selector` on iOS waits for the
picker on the main thread right after showing it (a deadlock), does not ask
for a copy of the file (`asCopy:`), and leaves `%20` in the paths it returns.
flint uses gpui's own `prompt_for_paths` instead, which gpui-mobile does right.

Sent as PRs, and on `flint` too:

- [#44](https://github.com/longbridge/gpui-mobile/pull/44): apps built with the
  iOS 27 SDK must use scenes, or they stop at launch on a phone. gpui-mobile
  made its `UIWindow` without a scene, so flint's `ios/main.m` has a scene
  delegate that moves the window onto the scene. The PR makes the window on the
  scene itself and gives the example a scene delegate.
- [#43](https://github.com/longbridge/gpui-mobile/pull/43): iOS reported each
  active status twice.

Also here: `android/.../GpuiInputActivity.java`, a copy of gpui-mobile's
file, asks the keyboard for no suggestions and knows the password keyboard.

When the fixes are merged, `Cargo.toml` takes gpui-mobile from git again.

## Code shared with flint-gpui

`src/ui/*` (except `terminal_view.rs`), `src/theme.rs` and
`src/preferences.rs` are copies of flint-gpui's files. `theme.rs` also sets
gpui-component's mono font, which on a phone is otherwise one it lacks. What
the phone needs on top of the shared palette, its interface font and the
surfaces of its screens, is in `src/phone_theme.rs`, so `theme.rs` stays a copy.

The interface is set in Inter (`assets/fonts/Inter-LICENSE.txt`, SIL Open Font
License); addresses, paths, keys and the terminal stay in Cascadia Code.

The icons are Lucide (`assets/icons/LUCIDE-LICENSE.txt`), and the system marks
in `assets/icons/distro/` are flint-gpui's, from Simple Icons (CC0). Once the experiment
works out, they move to a crate that both apps use.

## License

GPL-3.0. See [LICENSE](LICENSE).
