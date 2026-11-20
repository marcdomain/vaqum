use std::path::Path;

use vaqum::extract::Extracted;

/// Everything extracted must resolve to a location inside `dest`, and
/// nothing may appear in the scratch dir besides `out` and `staging`.
pub fn assert_contained(scratch: &Path, dest: &Path, ext: &Extracted) {
    let Ok(root) = dest.canonicalize() else {
        return;
    };
    for path in &ext.written {
        // Links may dangle; check where their parent folder really is.
        let check = if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            path.parent().map(Path::to_path_buf)
        } else {
            Some(path.clone())
        };
        if let Some(real) = check.and_then(|p| p.canonicalize().ok()) {
            assert!(real.starts_with(&root), "wrote outside the output dir: {path:?} -> {real:?}");
        }
    }
    for entry in std::fs::read_dir(scratch).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(name == "out" || name == "staging", "unexpected file in scratch dir: {name:?}");
    }
}
