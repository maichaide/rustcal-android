$ErrorActionPreference = 'Stop'
$NDK = "$env:LOCALAPPDATA\Android\Sdk\ndk\30.0.16248370"
$SDK = "$env:LOCALAPPDATA\Android\Sdk"
$BIN = "$NDK\toolchains\llvm\prebuilt\windows-x86_64\bin"
$API = 23
$CC = "$BIN\aarch64-linux-android$API-clang.cmd"

if (-not (Test-Path $CC)) {
    throw "Android NDK compiler wrapper not found: $CC"
}

$env:ANDROID_NDK_HOME = $NDK
$env:ANDROID_NDK_ROOT = $NDK
$env:ANDROID_SDK_ROOT = $SDK
[Environment]::SetEnvironmentVariable("CC_aarch64-linux-android", $CC, "Process")
$env:CC_aarch64_linux_android = $CC
$env:AR_aarch64_linux_android = "$BIN\llvm-ar.exe"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $CC
$env:PATH = "$BIN;$env:PATH"

rustup target add aarch64-linux-android
cargo apk build --target aarch64-linux-android --lib --release
