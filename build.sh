#!/usr/bin/env bash
# Builds flint-mobile and runs it.
#
#   ./build.sh ios-sim               build, install and start on the iOS Simulator
#   ./build.sh ios-sim --no-build    start the last simulator build again
#   ./build.sh ios-device            build, install and start on the connected iPhone
#   ./build.sh ios-device --no-build start the last iPhone build again
#   ./build.sh ios                   generate and open the Xcode project, to run and sign there
#   ./build.sh android               build, install and start on the Android phone adb sees
#   ./build.sh android --release     the same, with a release build of the Rust library
#
# Needs:
#   all       rustup (it installs the toolchain and targets)
#   iOS       Xcode
#             XcodeGen: brew install xcodegen
#   Android   the Android SDK and NDK
#             cargo-ndk: cargo install cargo-ndk

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bundle_id="com.mouse.flint"

step() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
fail() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null || fail "$1 is not installed. $2"
}

# Root-owned build files break the next normal build, and the simulator belongs to the user.
[[ $EUID -ne 0 ]] || fail "run it without sudo"

ios_project() {
    need xcodegen "Install it with: brew install xcodegen"
    step "Generating ios/Flint.xcodeproj"
    (cd "$root/ios" && xcodegen generate --spec project.yml --quiet)
}

ios_targets() {
    # rustup adds rust-toolchain.toml's targets only when it installs the toolchain.
    (cd "$root" && rustup target add aarch64-apple-ios aarch64-apple-ios-sim >/dev/null)
}

ios_sim() {
    local build_dir="$root/ios/build"
    local app="$build_dir/Build/Products/Debug-iphonesimulator/Flint.app"

    if [[ "${1:-}" != "--no-build" ]]; then
        ios_targets
        ios_project
        step "Building for the simulator (log: ios/build/xcodebuild.log)"
        local log="$build_dir/xcodebuild.log"
        mkdir -p "$build_dir"
        if ! xcodebuild \
            -project "$root/ios/Flint.xcodeproj" \
            -scheme Flint \
            -configuration Debug \
            -sdk iphonesimulator \
            -destination "generic/platform=iOS Simulator" \
            -derivedDataPath "$build_dir" \
            build >"$log" 2>&1; then
            grep -E "error(\[E[0-9]+\])?:|^error|BUILD FAILED" "$log" | head -30 || true
            fail "the build failed, see $log"
        fi
    fi
    [[ -d "$app" ]] || fail "no Flint.app in $build_dir"

    local sim
    sim=$(xcrun simctl list devices available | grep -m1 "iPhone" | sed -E 's/.*\(([A-F0-9-]+)\).*/\1/')
    [[ -n "$sim" ]] || fail "no iPhone simulator. Add one in Xcode: Window > Devices and Simulators."

    step "Starting on the simulator (log: ios/build/app.log)"
    xcrun simctl boot "$sim" 2>/dev/null || true
    # Xcode 27 shows simulators in Device Hub, older Xcodes in Simulator.app.
    local developer viewer
    developer="$(xcode-select -p)"
    for viewer in "$developer/../Applications/DeviceHub.app" "$developer/Applications/Simulator.app"; do
        if [[ -d "$viewer" ]]; then
            open "$viewer"
            break
        fi
    done
    xcrun simctl bootstatus "$sim" -b >/dev/null
    xcrun simctl install "$sim" "$app"
    xcrun simctl launch --console-pty --terminate-running-process "$sim" "$bundle_id" 2>&1 |
        tee "$build_dir/app.log"
}

# Prints the UDID, name and Developer Mode status of the first paired iPhone, tab-separated.
find_iphone() {
    # devicectl's text output changes between Xcode versions; its JSON does not.
    DEVICES_JSON="$(xcrun devicectl list devices --quiet --json-output - 2>/dev/null || true)" \
        python3 - <<'EOF'
import json
import os

try:
    devices = json.loads(os.environ["DEVICES_JSON"])["result"]["devices"]
except (ValueError, KeyError, TypeError):
    devices = []


def is_usable(device):
    hardware = device.get("hardwareProperties", {})
    connection = device.get("connectionProperties", {})
    return (
        hardware.get("platform") == "iOS"
        and hardware.get("reality", "physical") == "physical"
        and connection.get("pairingState") == "paired"
        and connection.get("transportType") in ("wired", "localNetwork")
    )


phones = sorted(
    filter(is_usable, devices),
    key=lambda device: device["connectionProperties"]["transportType"] != "wired",
)
if phones:
    phone = phones[0]
    properties = phone.get("deviceProperties", {})
    print(
        phone["hardwareProperties"]["udid"],
        properties.get("name", "iPhone"),
        properties.get("developerModeStatus", "unknown"),
        sep="\t",
    )
EOF
}

ios_device() {
    local build_dir="$root/ios/build"
    local app="$build_dir/Build/Products/Debug-iphoneos/Flint.app"

    local phone udid name developer_mode
    phone="$(find_iphone)"
    [[ -n "$phone" ]] ||
        fail "no iPhone found. Connect it to the Mac with a cable, unlock it and tap Trust."
    IFS=$'\t' read -r udid name developer_mode <<<"$phone"
    [[ "$developer_mode" != "disabled" ]] ||
        fail "Developer Mode is off on $name. Turn it on in Settings > Privacy & Security > Developer Mode (the phone restarts), then run this again."

    if [[ "${1:-}" != "--no-build" ]]; then
        ios_targets
        ios_project
        step "Building for $name (log: ios/build/xcodebuild-device.log)"
        echo "The first build compiles gpui for the phone and takes a few minutes."
        local build_log="$build_dir/xcodebuild-device.log"
        mkdir -p "$build_dir"
        # Lets Xcode create the signing profile and register this phone with the team.
        if ! xcodebuild \
            -project "$root/ios/Flint.xcodeproj" \
            -scheme Flint \
            -configuration Debug \
            -destination "platform=iOS,id=$udid" \
            -derivedDataPath "$build_dir" \
            -allowProvisioningUpdates \
            -allowProvisioningDeviceRegistration \
            build >"$build_log" 2>&1; then
            grep -E "error(\[E[0-9]+\])?:|^error|BUILD FAILED" "$build_log" | head -30 || true
            if grep -qE "No Account for Team|requires a development team|No profiles for|No signing certificate" "$build_log"; then
                echo "Sign in to Xcode with your Apple ID: Xcode > Settings > Accounts."
                echo "DEVELOPMENT_TEAM in ios/project.yml must be a team of that Apple ID."
            fi
            if grep -q "Unable to find a destination" "$build_log"; then
                echo "Xcode cannot build for $name yet. Open Xcode with the phone connected and"
                echo "wait until it has prepared the phone. If the phone has a newer iOS than"
                echo "this Xcode knows, update Xcode."
            fi
            fail "the build failed, see $build_log"
        fi
    fi
    [[ -d "$app" ]] || fail "no Flint.app in $build_dir"

    step "Installing on $name"
    local output
    if ! output="$(xcrun devicectl device install app --device "$udid" "$app" 2>&1)"; then
        echo "$output" | tail -15
        fail "could not install Flint. Keep the phone unlocked and run this again."
    fi

    step "Starting on $name (full log: ios/build/device.log, Ctrl+C stops the log)"
    local app_log="$build_dir/device.log"
    # System log lines, tagged [Category] or [], go only to the file; flint's are [INFO], [WARN].
    local system_line='^[0-9-]+ [0-9:.+]+ Flint\[[0-9:]+\] \[([^]]*[a-z][^]]*)?\] '
    local status=0
    # Ctrl+C is how the log ends; it is not an error.
    trap 'echo; exit 0' INT
    xcrun devicectl device process launch --console --terminate-existing \
        --device "$udid" "$bundle_id" 2>&1 |
        tee "$app_log" |
        { grep --line-buffered -vE "$system_line" || true; } ||
        status=$?
    [[ $status -ne 0 ]] || return 0

    if grep -qiE "explicitly trusted|untrusted|not trusted" "$app_log"; then
        echo "Trust your Apple ID on the phone once:"
        echo "Settings > General > VPN & Device Management > your Apple ID > Trust."
        fail "then run: ./build.sh ios-device --no-build"
    fi
    if grep -q "Launched application" "$app_log"; then
        fail "Flint stopped. The reason is near the end of ios/build/device.log"
    fi
    fail "could not start Flint. Unlock the phone and run: ./build.sh ios-device --no-build"
}

ios_xcode() {
    ios_targets
    ios_project
    step "Opening Xcode"
    echo "Pick your iPhone at the top of the window and press Run (Cmd+R)."
    echo "The first build compiles gpui and takes a few minutes."
    open "$root/ios/Flint.xcodeproj"
}

# The APK is always the debug-signed one; --release only optimizes the Rust library.
android() {
    local profile_flag=""
    if [[ "${1:-}" == "--release" ]]; then
        profile_flag="--release"
    fi
    need cargo-ndk "Install it with: cargo install cargo-ndk"
    need adb "It comes with the Android SDK platform-tools."

    step "Building the Rust library for arm64-v8a"
    (cd "$root" && cargo ndk -t arm64-v8a -P 26 \
        -o android/app/src/main/jniLibs build --lib $profile_flag)

    step "Building the APK"
    (cd "$root/android" && ./gradlew --quiet assembleDebug)
    local apk="$root/android/app/build/outputs/apk/debug/app-debug.apk"

    step "Installing and starting"
    adb install -r "$apk"
    adb shell am start -n "$bundle_id/dev.gpui.mobile.GpuiActivity"
    echo "Logs: adb logcat -s flint:V gpui-mobile:V"
}

case "${1:-}" in
    ios-sim) shift; ios_sim "$@" ;;
    ios-device) shift; ios_device "$@" ;;
    ios) ios_xcode ;;
    android) shift; android "$@" ;;
    *) sed -n '2,17p' "$0"; exit 1 ;;
esac
