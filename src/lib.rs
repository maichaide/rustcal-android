#![allow(clippy::needless_return)]

pub mod app;
pub mod db;
pub mod models;
pub mod sync;
pub mod ui;
pub mod platform;

use app::RustCalApp;
use eframe::egui;

pub fn run_desktop() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 480.0])
            .with_title("RustCal - 日历与任务管理"),
        ..Default::default()
    };

    eframe::run_native(
        "RustCal",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let font_loaded = setup_fonts(&cc.egui_ctx);
            Ok(Box::new(RustCalApp::new(cc, font_loaded)))
        }),
    )
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub fn android_main(app: winit::platform::android::activity::AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info),
    );

    // AndroidApp supplies the real sandbox path. Keep all persistent RustCal data
    // under the app-private files directory; never depend on process cwd.
    if let Some(base) = app.internal_data_path() {
        let data_dir = base.join("rustcal");
        if let Err(error) = std::fs::create_dir_all(&data_dir) {
            log::error!("Cannot create RustCal data directory {:?}: {error}", data_dir);
        }
        // SAFETY: this is called once at Android startup, before the app creates worker threads.
        unsafe { std::env::set_var("RUSTCAL_DATA_DIR", data_dir) };
    } else {
        log::error!("Android did not provide an internal data path");
    }

    let options = eframe::NativeOptions {
        android_app: Some(app),
        ..Default::default()
    };

    if let Err(error) = eframe::run_native(
        "RustCal",
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let font_loaded = setup_fonts(&cc.egui_ctx);
            Ok(Box::new(RustCalApp::new(cc, font_loaded)))
        }),
    ) {
        log::error!("RustCal Android startup failed: {error}");
    }
}

/// Load a system CJK font where available. Android's system font directory is
/// checked explicitly; egui's bundled default font remains the fallback.
fn setup_fonts(ctx: &egui::Context) -> bool {
    let mut fonts = egui::FontDefinitions::default();
    let candidate_paths = [
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "/System/Library/Fonts/PingFang.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/system/fonts/NotoSansCJK-Regular.ttc",
        "/system/fonts/NotoSansSC-VF.ttf",
        "/system/fonts/NotoSansCJKsc-Regular.otf",
    ];

    for path in candidate_paths {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk_primary".to_owned(),
                egui::FontData::from_owned(data).into(),
            );
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "cjk_primary".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.insert(0, "cjk_primary".to_owned());
            }
            ctx.set_fonts(fonts);
            return true;
        }
    }
    false
}
