use super::*;
use anyhow::bail;

/// Copy each installed mod map's depot file into vehicle folders that already contain a
/// vehicle-local HOF. OMSI expects that copy next to every bus; openOMSI keeps it in its
/// writable content folder so the original installation stays untouched.
#[derive(Default)]
struct MapHofSyncState {
    stamp: Option<String>,
    warnings: Vec<String>,
}

fn map_hof_sync_state() -> &'static std::sync::Mutex<MapHofSyncState> {
    static STATE: std::sync::OnceLock<std::sync::Mutex<MapHofSyncState>> = std::sync::OnceLock::new();
    STATE.get_or_init(Default::default)
}

pub(super) fn sync_map_hofs_to_vehicle_folders() {
    let Some(content) = content_dir() else { return };
    let Ok(original) = root() else { return };
    let canonical_content = std::fs::canonicalize(&content).unwrap_or_else(|_| content.clone());
    let canonical_original = std::fs::canonicalize(&original).unwrap_or(original);
    if canonical_content == canonical_original {
        return;
    }
    let roots = omsi_cfg::content_roots();
    let mut stamp = index::content_stamp(&roots, None);
    let hof_dirs: Vec<PathBuf> = roots.iter().map(|r| r.join("HOFs")).collect();
    stamp.push_str(&format!("-{:016x}", index::folder_stamp(&hof_dirs)));
    let state = map_hof_sync_state();
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    if state.stamp.as_deref() == Some(stamp.as_str()) {
        return;
    }

    // Invalidate missed depot lookups when a map or HOF was added outside the installer.
    omsi_cfg::content_changed();

    let mut vehicles = Vec::new();
    for dir in omsi_cfg::read_dir_merged("Vehicles") {
        if !omsi_cfg::vfs::is_dir(&dir) {
            continue;
        }
        let name = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mirrors = omsi_cfg::mirrored_dirs(&dir);
        let mut local_hofs = std::collections::HashSet::new();
        for mirror in mirrors {
            for (file, is_dir) in omsi_cfg::vfs::list_dir(&mirror).unwrap_or_default() {
                if !is_dir && Path::new(&file).extension().is_some_and(|e| e.eq_ignore_ascii_case("hof")) {
                    local_hofs.insert(file.to_string_lossy().to_ascii_lowercase());
                }
            }
        }
        if !local_hofs.is_empty() {
            vehicles.push((name, local_hofs));
        }
    }

    let mut copied = 0usize;
    let mut retry = false;
    let mut warnings = Vec::new();
    let mut seen_hofs = std::collections::HashSet::new();
    for (folder, dirs) in merged_folders("maps") {
        let Some(map) = dirs.into_iter().find(|d| in_content(d) && omsi_cfg::vfs::is_file(&d.join("global.cfg"))) else { continue };
        let ailists_path = map.join("ailists.cfg");
        if !omsi_cfg::vfs::is_file(&ailists_path) {
            continue;
        }
        let ailists = match omsi_map::ailists::AiLists::load(&ailists_path) {
            Ok(ailists) => ailists,
            Err(e) => {
                let warning = format!("{folder}: couldn't read ailists.cfg: {e}");
                log_line(&format!("maps: {warning}"));
                warnings.push(warning);
                retry = true;
                continue;
            }
        };
        for name in ailists.groups.iter().filter_map(|g| g.hof.as_deref()).map(str::trim).filter(|n| !n.is_empty()) {
            let name = name.rsplit(['\\', '/']).next().unwrap_or(name);
            let name = if name.get(name.len().saturating_sub(4)..).is_some_and(|ext| ext.eq_ignore_ascii_case(".hof")) {
                &name[..name.len() - 4]
            } else {
                name
            };
            if !seen_hofs.insert(name.to_ascii_lowercase()) {
                continue;
            }
            let Some(hof) = omsi_vehicle::hof::depot_anywhere(name) else {
                warnings.push(format!("{folder}: couldn't find the required HOF file '{name}.hof' in the installed content."));
                continue;
            };
            let Some(file_name) = hof.path.file_name() else { continue };
            let file_name = file_name.to_string_lossy().to_string();
            let key = file_name.to_ascii_lowercase();
            let bytes = match omsi_cfg::vfs::read(&hof.path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    let warning = format!("{folder}: couldn't read HOF {}: {e}", hof.path.display());
                    log_line(&format!("maps: {warning}"));
                    warnings.push(warning);
                    retry = true;
                    continue;
                }
            };
            for (vehicle, local_hofs) in &mut vehicles {
                if local_hofs.contains(&key) {
                    continue;
                }
                let rel = format!("Vehicles/{vehicle}/{file_name}");
                let dest = content_case_path(&content, &rel);
                if !path_stays_in_content(&content, dest.parent().unwrap_or(&content)) {
                    let warning = format!("{folder}: couldn't add {file_name} to {vehicle}; the destination is outside the writable content folder.");
                    log_line(&format!("maps: {warning}"));
                    warnings.push(warning);
                    retry = true;
                    continue;
                }
                if let Some(parent) = dest.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        let warning = format!("{folder}: couldn't create the folder for {file_name} in {vehicle}: {e}");
                        log_line(&format!("maps: {warning}"));
                        warnings.push(warning);
                        retry = true;
                        continue;
                    }
                }
                let mut output = match std::fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
                    Ok(file) => file,
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        local_hofs.insert(key.clone());
                        continue;
                    }
                    Err(e) => {
                        let warning = format!("{folder}: couldn't add {file_name} to {vehicle}: {e}");
                        log_line(&format!("maps: {warning}"));
                        warnings.push(warning);
                        retry = true;
                        continue;
                    }
                };
                if let Err(e) = std::io::Write::write_all(&mut output, &bytes) {
                    let _ = std::fs::remove_file(&dest);
                    let warning = format!("{folder}: couldn't finish adding {file_name} to {vehicle}: {e}");
                    log_line(&format!("maps: {warning}"));
                    warnings.push(warning);
                    retry = true;
                    continue;
                }
                local_hofs.insert(key.clone());
                copied += 1;
            }
        }
    }

    if copied > 0 {
        log_line(&format!("maps: added {copied} map HOF file(s) to existing vehicle folders in {}", content.display()));
        omsi_cfg::content_changed();
    }
    warnings.sort();
    warnings.dedup();
    if !warnings.is_empty() {
        for warning in &warnings {
            log_line(&format!("maps: HOF sync warning: {warning}"));
        }
        log_line(&format!("maps: HOF sync has {} warning(s); see the Mods page", warnings.len()));
    }

    // Store the post-copy stamp, so new overlay files don't cause another full scan.
    let roots = omsi_cfg::content_roots();
    let mut final_stamp = index::content_stamp(&roots, None);
    let hof_dirs: Vec<PathBuf> = roots.iter().map(|r| r.join("HOFs")).collect();
    final_stamp.push_str(&format!("-{:016x}", index::folder_stamp(&hof_dirs)));
    state.warnings = warnings;
    state.stamp = if retry { None } else { Some(final_stamp) };
}

/// Copy player-selected depot files into vehicle folders that already have local HOFs.
/// Only the writable openOMSI content folder is changed; existing files are left alone.
pub fn add_hofs_to_vehicle_folders(files: &[PathBuf]) -> Result<String> {
    let content = content_dir().ok_or_else(|| anyhow!("no game content folder is configured"))?;
    let original = root()?;
    let canonical_content = std::fs::canonicalize(&content).unwrap_or_else(|_| content.clone());
    let canonical_original = std::fs::canonicalize(&original).unwrap_or(original);
    if canonical_content == canonical_original {
        bail!("the writable content folder is the original OMSI folder; HOF files were not copied");
    }
    if files.is_empty() {
        bail!("no HOF files were selected");
    }

    let mut vehicles = Vec::new();
    for dir in omsi_cfg::read_dir_merged("Vehicles") {
        if !omsi_cfg::vfs::is_dir(&dir) { continue; }
        let name = dir.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mut local_hofs = std::collections::HashSet::new();
        for mirror in omsi_cfg::mirrored_dirs(&dir) {
            for (file, is_dir) in omsi_cfg::vfs::list_dir(&mirror).unwrap_or_default() {
                if !is_dir && Path::new(&file).extension().is_some_and(|e| e.eq_ignore_ascii_case("hof")) {
                    local_hofs.insert(file.to_string_lossy().to_ascii_lowercase());
                }
            }
        }
        if !local_hofs.is_empty() { vehicles.push((name, local_hofs)); }
    }
    if vehicles.is_empty() {
        bail!("no vehicle folders with existing HOF files were found");
    }

    let mut copied = 0usize;
    let mut skipped = 0usize;
    let mut failures = Vec::new();
    for source in files {
        if !source.extension().is_some_and(|e| e.eq_ignore_ascii_case("hof")) {
            failures.push(format!("{} is not a .hof file", source.display()));
            continue;
        }
        let Some(file_name) = source.file_name() else {
            failures.push(format!("{} has no filename", source.display()));
            continue;
        };
        let file_name = file_name.to_string_lossy().to_string();
        let key = file_name.to_ascii_lowercase();
        let bytes = match std::fs::read(source) {
            Ok(bytes) => bytes,
            Err(e) => { failures.push(format!("couldn't read {file_name}: {e}")); continue; }
        };
        for (vehicle, local_hofs) in &mut vehicles {
            if local_hofs.contains(&key) { skipped += 1; continue; }
            let dest = content_case_path(&content, &format!("Vehicles/{vehicle}/{file_name}"));
            if !path_stays_in_content(&content, dest.parent().unwrap_or(&content)) {
                failures.push(format!("couldn't safely add {file_name} to {vehicle}"));
                continue;
            }
            if let Some(parent) = dest.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    failures.push(format!("couldn't prepare {vehicle} for {file_name}: {e}"));
                    continue;
                }
            }
            let mut output = match std::fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
                Ok(file) => file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => { local_hofs.insert(key.clone()); skipped += 1; continue; }
                Err(e) => { failures.push(format!("couldn't add {file_name} to {vehicle}: {e}")); continue; }
            };
            if let Err(e) = std::io::Write::write_all(&mut output, &bytes) {
                let _ = std::fs::remove_file(&dest);
                failures.push(format!("couldn't finish adding {file_name} to {vehicle}: {e}"));
                continue;
            }
            local_hofs.insert(key.clone());
            copied += 1;
        }
    }
    if copied > 0 { omsi_cfg::content_changed(); }
    if !failures.is_empty() {
        bail!("copied {copied} file(s), skipped {skipped} existing copy/copies; {} issue(s): {}", failures.len(), failures.join("; "));
    }
    if copied == 0 {
        bail!("all selected HOF files were already present in the eligible vehicle folders");
    }
    Ok(format!("copied {copied} HOF file(s) to bus folders; skipped {skipped} existing copy/copies"))
}

/// Resolve each component case-insensitively inside the writable content folder only.
fn content_case_path(base: &Path, rel: &str) -> PathBuf {
    let mut current = base.to_path_buf();
    for component in rel.split('/').filter(|c| !c.is_empty()) {
        let direct = current.join(component);
        if direct.exists() {
            current = direct;
            continue;
        }
        let found = std::fs::read_dir(&current).ok().and_then(|rd| rd.flatten().map(|e| e.file_name()).find(|n| n.to_string_lossy().eq_ignore_ascii_case(component)));
        current = found.map(|name| current.join(name)).unwrap_or(direct);
    }
    current
}

/// Refuse to follow a link out of the writable overlay into another content root.
fn path_stays_in_content(content: &Path, path: &Path) -> bool {
    let base = std::fs::canonicalize(content).unwrap_or_else(|_| content.to_path_buf());
    let mut ancestor = path;
    while !ancestor.exists() {
        let Some(parent) = ancestor.parent() else { return false };
        ancestor = parent;
    }
    std::fs::canonicalize(ancestor).map(|p| p.starts_with(base)).unwrap_or(false)
}


pub(super) fn warnings() -> Vec<String> {
    map_hof_sync_state().lock().unwrap_or_else(|e| e.into_inner()).warnings.clone()
}
