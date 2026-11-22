use std::path::PathBuf;

/// Persistent data root shared by the database and configuration files.
pub fn data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("RUSTCAL_DATA_DIR") {
        return PathBuf::from(path);
    }

    #[cfg(not(target_os = "android"))]
    {
        if let Some(project) = directories::ProjectDirs::from("io", "rustcal", "RustCal") {
            return project.data_dir().to_path_buf();
        }
    }

    // A deterministic fallback for unusual environments. Android normally sets
    // RUSTCAL_DATA_DIR in android_main before constructing the application.
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir)
        .join("RustCal")
}

pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "android")]
    {
        data_dir().join("config")
    }
    #[cfg(not(target_os = "android"))]
    {
        directories::ProjectDirs::from("io", "rustcal", "RustCal")
            .map(|p| p.config_dir().to_path_buf())
            .unwrap_or_else(|| data_dir().join("config"))
    }
}

pub fn import_dir() -> PathBuf {
    data_dir().join("import")
}

pub fn export_dir() -> PathBuf {
    data_dir().join("export")
}

pub fn ensure_user_dirs() -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir())?;
    std::fs::create_dir_all(config_dir())?;
    std::fs::create_dir_all(import_dir())?;
    std::fs::create_dir_all(export_dir())?;
    Ok(())
}
