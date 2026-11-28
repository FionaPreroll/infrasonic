// SPDX-License-Identifier: GPL-3.0-only
//! The presets of the collections the visualizer reads.

use std::path::{Path, PathBuf};

/// Preset collections: each a directory with `presets/` (`.milk` files, subfolders too) and `textures/`. Later
/// collections win when two hold a texture of the same name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Collections {
    roots: Vec<PathBuf>,
    presets: Vec<PathBuf>,
}

/// Presets one collection may hold: the bundles hold about 9,000.
const MAX_PRESETS: usize = 50_000;
/// Directory levels searched below `presets/` (the bundles use one level of categories).
const MAX_DEPTH: usize = 4;
/// Largest preset file read; the largest of the Cream of the Crop collection has 120 KB.
pub(crate) const MAX_PRESET_BYTES: u64 = 1 << 20;

impl Collections {
    /// Find the presets of `roots` now; directories that do not exist are skipped.
    pub fn scan(roots: &[PathBuf]) -> Self {
        let mut presets = Vec::new();
        for root in roots {
            find_presets(&root.join("presets"), 0, &mut presets);
        }
        presets.sort();
        presets.dedup();
        presets.truncate(MAX_PRESETS);
        Self {
            roots: roots.to_vec(),
            presets,
        }
    }

    pub fn presets(&self) -> &[PathBuf] {
        &self.presets
    }

    /// Texture directories, the one that wins first.
    pub fn texture_dirs(&self) -> Vec<PathBuf> {
        self.roots
            .iter()
            .rev()
            .map(|r| r.join("textures"))
            .filter(|d| d.is_dir())
            .collect()
    }
}

fn find_presets(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        // no symlinks: a collection is a plain directory tree the app wrote itself
        let Ok(kind) = e.file_type() else { continue };
        let path = e.path();
        if kind.is_dir() && depth < MAX_DEPTH {
            find_presets(&path, depth + 1, out);
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("milk"))
            && e.metadata().is_ok_and(|m| m.len() <= MAX_PRESET_BYTES)
        {
            out.push(path);
            if out.len() >= MAX_PRESETS {
                return;
            }
        }
    }
}

/// Display name of a preset: the file name without `.milk`.
pub(crate) fn preset_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path, bytes: usize) {
        std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
        std::fs::write(p, vec![b'x'; bytes]).expect("write");
    }

    #[test]
    fn finds_presets_in_category_folders_of_every_collection() {
        let a = tempfile::tempdir().expect("tmp");
        let b = tempfile::tempdir().expect("tmp");
        touch(&a.path().join("presets/one.milk"), 10);
        touch(&a.path().join("presets/Fractal/two.MILK"), 10);
        touch(&a.path().join("presets/notes.txt"), 10);
        touch(&b.path().join("presets/Dancer/three.milk"), 10);
        touch(
            &b.path().join("presets/huge.milk"),
            MAX_PRESET_BYTES as usize + 1,
        );
        std::fs::create_dir_all(b.path().join("textures")).expect("mkdir");
        let c = Collections::scan(&[a.path().into(), b.path().into(), "/nonexistent".into()]);
        let names: Vec<String> = c.presets().iter().map(|p| preset_name(p)).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.contains(&"two".to_string()) && names.contains(&"three".to_string()));
        // only b has textures; the last collection comes first
        assert_eq!(c.texture_dirs(), vec![b.path().join("textures")]);
    }
}
