use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

pub fn library_path() -> Result<PathBuf, String> {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .map(|path| path.join("Quby Seemdissolver").join("Quby Materials"))
        .ok_or_else(|| {
            "Could not locate the Windows Local AppData folder for Quby Materials.".into()
        })
}

fn is_asset(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("blend"))
        && path
            .file_name()
            .is_some_and(|name| !name.to_string_lossy().starts_with(".quby-"))
}

pub fn list() -> Result<Vec<PathBuf>, String> {
    let root = library_path()?;
    let read = || -> io::Result<Vec<PathBuf>> {
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut assets = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if is_asset(&path) && entry.file_type()?.is_file() {
                assets.push(path);
            }
        }
        assets.sort_by_key(|path| path.to_string_lossy().to_lowercase());
        Ok(assets)
    };
    read().map_err(|error| format!("Could not read {}: {error}", root.display()))
}

pub fn recycle(path: &Path) -> Result<(), String> {
    let root = library_path()?;
    if path.parent() != Some(root.as_path()) || !is_asset(path) {
        return Err("This file is outside the Quby Materials library.".into());
    }
    let kind = fs::symlink_metadata(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?
        .file_type();
    if !kind.is_file() {
        return Err("Only Blender material files can be removed.".into());
    }
    recycle_file(path)
}

#[cfg(windows)]
fn recycle_file(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI,
        FOF_NO_CONNECTED_ELEMENTS, FOF_SILENT, FOF_WANTNUKEWARNING, FO_DELETE, SHFILEOPSTRUCTW,
    };

    let from: Vec<_> = path.as_os_str().encode_wide().chain([0, 0]).collect();
    let mut operation = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: (FOF_ALLOWUNDO
            | FOF_NOCONFIRMATION
            | FOF_NO_CONNECTED_ELEMENTS
            | FOF_NOERRORUI
            | FOF_SILENT
            | FOF_WANTNUKEWARNING) as u16,
        ..Default::default()
    };
    // The source is one absolute path, double null terminated and alive for the call.
    let code = unsafe { SHFileOperationW(&mut operation) };
    if operation.fAnyOperationsAborted != 0 {
        return Err("Windows cancelled the removal.".into());
    }
    if code != 0 {
        return Err(format!(
            "Could not move {} to the Recycle Bin (Windows shell error {code}).",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn recycle_file(_: &Path) -> Result<(), String> {
    Err("The Recycle Bin is available on Windows.".into())
}
