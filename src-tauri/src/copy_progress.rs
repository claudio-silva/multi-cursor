use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::paths::move_to_trash;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyProgressEvent {
    pub percent: u8,
    pub label: String,
}

pub fn emit_progress(app: &AppHandle, percent: u8, label: impl Into<String>) {
    let _ = app.emit(
        "env-copy-progress",
        CopyProgressEvent {
            percent: percent.min(100),
            label: label.into(),
        },
    );
}

fn map_into_range(local: u8, range_start: u8, range_end: u8) -> u8 {
    let start = u16::from(range_start.min(100));
    let end = u16::from(range_end.min(100).max(range_start));
    let span = end.saturating_sub(start);
    (start + u16::from(local.min(100)) * span / 100) as u8
}

fn friendly_copy_error(err: &std::io::Error, path: &Path) -> String {
    let msg = err.to_string();
    let lower = msg.to_lowercase();
    if err.raw_os_error() == Some(28) || lower.contains("no space") {
        return "Not enough disk space to copy the environment. Free some space and try again."
            .to_string();
    }
    if lower.contains("permission denied") {
        return format!(
            "Permission denied while copying {}: check folder permissions and try again.",
            path.display()
        );
    }
    format!("Copy failed at {}: {msg}", path.display())
}

pub fn cleanup_partial_copy(dst: &Path) {
    if dst.exists() {
        let _ = std::fs::remove_dir_all(dst);
    }
}

pub fn purge_vscdb_bak_files(global_storage: &Path) -> u64 {
    let mut removed = 0u64;
    let Ok(entries) = std::fs::read_dir(global_storage) else {
        return 0;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("state.vscdb.bak-") || name == "state.vscdb.backup" {
            if let Ok(meta) = entry.metadata() {
                removed = removed.saturating_add(meta.len());
            }
            let _ = std::fs::remove_file(entry.path());
        }
    }
    removed
}

pub fn dir_size_bytes(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    let output = Command::new("du")
        .args(["-sk", &path.display().to_string()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output();
    let Ok(out) = output else {
        return 0;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let kb: u64 = text
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    kb.saturating_mul(1024)
}

fn is_excluded_dir_name(name: &str) -> bool {
    matches!(
        name,
        "Cache"
            | "CachedData"
            | "Code Cache"
            | "GPUCache"
            | "DawnCache"
            | "DawnWebGPUCache"
            | "DawnGraphiteCache"
            | "ShaderCache"
            | "logs"
            | "Crashpad"
    )
}

fn is_excluded_file_name(name: &str) -> bool {
    name == "state.vscdb.backup"
        || name.starts_with("state.vscdb.bak-")
        || name.contains(".bak-")
}

/// Skip heavy/cache paths and Cursor DB bak side-files.
fn should_skip_entry(path: &Path, is_dir: bool) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if is_dir {
        if is_excluded_dir_name(name) {
            return true;
        }
        // Match former rsync exclude: Service Worker/CacheStorage
        if name == "CacheStorage" {
            let parent = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str());
            return parent == Some("Service Worker");
        }
        return false;
    }
    is_excluded_file_name(name)
}

fn count_files_to_copy(src: &Path) -> Result<u64, String> {
    let mut count = 0u64;
    let mut stack = vec![src.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| friendly_copy_error(&e, &dir))?;
        for entry in entries {
            let entry = entry.map_err(|e| friendly_copy_error(&e, &dir))?;
            let path = entry.path();
            let meta = entry
                .metadata()
                .map_err(|e| friendly_copy_error(&e, &path))?;
            if meta.is_dir() {
                if should_skip_entry(&path, true) {
                    continue;
                }
                stack.push(path);
            } else if meta.is_file() {
                if should_skip_entry(&path, false) {
                    continue;
                }
                count += 1;
            }
            // Skip symlinks and special files.
        }
    }
    Ok(count)
}

fn copy_file(src: &Path, dst: &Path) -> Result<(), String> {
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| friendly_copy_error(&e, parent))?;
    }
    std::fs::copy(src, dst).map_err(|e| friendly_copy_error(&e, src))?;
    // Preserve permissions best-effort (macOS).
    if let Ok(meta) = std::fs::metadata(src) {
        let perms = meta.permissions();
        let _ = std::fs::set_permissions(dst, perms);
    }
    Ok(())
}

fn copy_tree_native(src: &Path, dst: &Path, files_done: &AtomicU64) -> Result<(), String> {
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src.to_path_buf(), dst.to_path_buf())];

    while let Some((src_dir, dst_dir)) = stack.pop() {
        std::fs::create_dir_all(&dst_dir).map_err(|e| friendly_copy_error(&e, &dst_dir))?;
        let entries =
            std::fs::read_dir(&src_dir).map_err(|e| friendly_copy_error(&e, &src_dir))?;
        for entry in entries {
            let entry = entry.map_err(|e| friendly_copy_error(&e, &src_dir))?;
            let src_path = entry.path();
            let file_name = entry.file_name();
            let dst_path = dst_dir.join(&file_name);
            let meta = entry
                .metadata()
                .map_err(|e| friendly_copy_error(&e, &src_path))?;

            if meta.is_dir() {
                if should_skip_entry(&src_path, true) {
                    continue;
                }
                stack.push((src_path, dst_path));
                continue;
            }
            if !meta.is_file() || should_skip_entry(&src_path, false) {
                continue;
            }

            copy_file(&src_path, &dst_path)?;
            files_done.fetch_add(1, Ordering::Relaxed);
        }
    }
    Ok(())
}

pub fn copy_cursor_tree_with_progress(
    app: &AppHandle,
    src: &Path,
    dst: &Path,
    label: &str,
    range_start: u8,
    range_end: u8,
) -> Result<(), String> {
    if !src.exists() {
        return Err(format!("Source does not exist: {}", src.display()));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    cleanup_partial_copy(dst);
    std::fs::create_dir_all(dst).map_err(|e| {
        let msg = e.to_string();
        if e.raw_os_error() == Some(28) || msg.to_lowercase().contains("no space") {
            "Not enough disk space to create the environment folder.".to_string()
        } else {
            msg
        }
    })?;

    let _ = purge_vscdb_bak_files(&src.join("User").join("globalStorage"));

    let src_size = dir_size_bytes(src);
    if src_size < 100_000 {
        return Err(format!(
            "Source looks nearly empty ({} bytes): {}. Nothing useful to copy.",
            src_size,
            src.display()
        ));
    }

    emit_progress(app, map_into_range(0, range_start, range_end), label);
    emit_progress(
        app,
        map_into_range(0, range_start, range_end),
        format!("{label} (counting files…)"),
    );

    let total_files = count_files_to_copy(src)?;
    if total_files == 0 {
        return Err(format!(
            "No files to copy from {} (everything excluded or empty).",
            src.display()
        ));
    }

    emit_progress(
        app,
        map_into_range(0, range_start, range_end),
        format!("{label} (0 / {total_files} files)"),
    );

    let files_done = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Background ticker so the bar still moves if a single huge file stalls the loop.
    let ticker_app = app.clone();
    let ticker_label = label.to_string();
    let ticker_done = Arc::clone(&files_done);
    let ticker_stop = Arc::clone(&stop);
    let ticker = thread::spawn(move || {
        let mut last = 0u64;
        while !ticker_stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));
            let done = ticker_done.load(Ordering::Relaxed);
            if done == last {
                continue;
            }
            last = done;
            let local = ((done.saturating_mul(100)) / total_files).min(99) as u8;
            emit_progress(
                &ticker_app,
                map_into_range(local, range_start, range_end),
                format!("{ticker_label} ({done} / {total_files} files)"),
            );
        }
    });

    let result = copy_tree_native(src, dst, &files_done);
    stop.store(true, Ordering::Relaxed);
    let _ = ticker.join();

    if let Err(err) = result {
        cleanup_partial_copy(dst);
        return Err(err);
    }

    let copied = dir_size_bytes(dst);
    let min_ok = src_size.saturating_div(50).max(1_000_000).min(src_size);
    if copied < min_ok {
        cleanup_partial_copy(dst);
        return Err(format!(
            "Copy finished but destination is too small ({copied} bytes vs source {src_size}). Try again."
        ));
    }

    let done = files_done.load(Ordering::Relaxed);
    emit_progress(
        app,
        map_into_range(100, range_start, range_end),
        format!("{label} ({done} / {total_files} files)"),
    );
    Ok(())
}

/// Count every regular file under `path` (no cache exclusions — used for delete).
/// Emits progress about once per second while counting when `app` is provided.
fn count_all_files(app: Option<&AppHandle>, path: &Path, running_total: &mut u64) -> u64 {
    if !path.exists() {
        return 0;
    }
    let mut count = 0u64;
    let mut stack = vec![path.to_path_buf()];
    let mut last_emit = std::time::Instant::now()
        .checked_sub(Duration::from_secs(2))
        .unwrap_or_else(std::time::Instant::now);
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(p);
            } else if meta.is_file() {
                count += 1;
                *running_total = running_total.saturating_add(1);
                if let Some(app) = app {
                    if last_emit.elapsed() >= Duration::from_secs(1) {
                        last_emit = std::time::Instant::now();
                        emit_progress(
                            app,
                            0,
                            format!("Counting files… ({running_total})"),
                        );
                    }
                }
            }
        }
    }
    count
}

fn emit_delete_progress(app: &AppHandle, done: u64, total: u64, label: &str) {
    let percent = if total == 0 {
        100
    } else {
        ((done.saturating_mul(100)) / total).min(99) as u8
    };
    emit_progress(
        app,
        percent,
        format!("{label} ({done} / {total} files)"),
    );
}

/// Move inactive environment folders to Trash, with file-count–weighted progress.
/// `trash`/`Finder` do not report per-file progress, so each root is weighted by
/// its file count and a 1s ticker advances within that slice while Trash runs.
pub fn trash_paths_with_progress(
    app: &AppHandle,
    paths: &[(PathBuf, String)],
) -> Result<(), String> {
    emit_progress(app, 0, "Counting files…");

    let mut counts: Vec<u64> = Vec::with_capacity(paths.len());
    let mut total = 0u64;
    for (path, _) in paths {
        let n = if path.exists() {
            count_all_files(Some(app), path, &mut total)
        } else {
            0
        };
        counts.push(n);
    }

    // Still trash empty / tiny dirs; treat missing totals as equal stages.
    let progress_total = total.max(1);
    let mut done_before = 0u64;

    for (i, (path, label)) in paths.iter().enumerate() {
        if !path.exists() {
            continue;
        }
        let weight = counts[i].max(1);
        let slice_start = ((done_before.saturating_mul(100)) / progress_total) as u8;
        let slice_end =
            (((done_before.saturating_add(weight)).saturating_mul(100)) / progress_total).min(99)
                as u8;

        emit_delete_progress(app, done_before, progress_total, label);

        let stop = Arc::new(AtomicBool::new(false));
        let ticker_app = app.clone();
        let ticker_label = label.clone();
        let ticker_stop = Arc::clone(&stop);
        let start_done = done_before;
        let ticker = thread::spawn(move || {
            let mut step = 0u64;
            while !ticker_stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(1));
                if ticker_stop.load(Ordering::Relaxed) {
                    break;
                }
                step = step.saturating_add(1);
                // Ease toward the end of this slice while Trash is working.
                let local = (step.saturating_mul(90) / (step + 3)).min(90) as u8;
                let percent = map_into_range(local, slice_start, slice_end.max(slice_start + 1));
                emit_progress(
                    &ticker_app,
                    percent,
                    format!("{ticker_label} ({start_done} / {progress_total} files)"),
                );
            }
        });

        let result = move_to_trash(path);
        stop.store(true, Ordering::Relaxed);
        let _ = ticker.join();
        result?;

        done_before = done_before.saturating_add(counts[i]);
        emit_delete_progress(app, done_before.min(progress_total), progress_total, label);
    }

    emit_progress(app, 100, "Moved to Trash");
    Ok(())
}
