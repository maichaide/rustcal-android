# RustCal

RustCal is a Rust/egui calendar and task manager. This source tree preserves the existing desktop application and adds an Android `NativeActivity` build target.

- Desktop build and usage: `cargo run`
- Android build notes: see [README-ANDROID.md](README-ANDROID.md)

The Android adaptation currently uses eframe/egui 0.31.1. Android support was added to eframe in 0.30.0. The Android target uses the `android-native-activity` backend.
