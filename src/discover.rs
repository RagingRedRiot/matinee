//! Finding a project's storyboards, so `matinee record` can record every
//! demo under a folder.
//!
//! A storyboard is recognized by what it holds, not by its name: a TOML
//! file with a `[demo]` table and at least one `[[scene]]`. Other TOML
//! files beside it, such as an app's own config or input files a demo
//! uses, are left alone. Hidden folders, `target`, and `node_modules` are
//! not searched, and symbolic links to folders are not followed.

use std::{
    io,
    path::{Path, PathBuf},
};

/// Folders never searched: build output and installed dependencies.
const SKIPPED: [&str; 2] = ["target", "node_modules"];

/// Whether `text` is a storyboard: TOML with a `[demo]` table and at
/// least one `[[scene]]`. It may still have mistakes; loading it reports
/// them.
pub fn is_storyboard(text: &str) -> bool {
    let Ok(table) = text.parse::<toml::Table>() else {
        return false;
    };
    table.get("demo").is_some_and(toml::Value::is_table)
        && table
            .get("scene")
            .and_then(toml::Value::as_array)
            .is_some_and(|scenes| !scenes.is_empty())
}

/// The storyboards under `root`, in path order. A file is taken as given,
/// whatever it holds, so its mistakes are reported rather than skipped.
pub fn storyboards(root: &Path) -> io::Result<Vec<PathBuf>> {
    let meta = std::fs::metadata(root)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", root.display())))?;
    if !meta.is_dir() {
        return Ok(vec![root.to_path_buf()]);
    }
    let mut found = vec![];
    search(root, &mut found)?;
    found.sort();
    Ok(found)
}

fn search(dir: &Path, found: &mut Vec<PathBuf>) -> io::Result<()> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `file_type` does not follow symbolic links.
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if !name.starts_with('.') && !SKIPPED.contains(&name.as_ref()) {
                search(&path, found)?;
            }
        } else if name.ends_with(".toml")
            && std::fs::read_to_string(&path).is_ok_and(|text| is_storyboard(&text))
        {
            found.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORY: &str = "[demo]\noutput = \"d\"\nsize = [640, 360]\n\
                         [[scene]]\nsteps = [{ run = \"true\" }]\n";

    #[test]
    fn storyboards_are_known_by_their_tables() {
        assert!(is_storyboard(STORY));
        // A pipeline file a demo submits, and a Cargo manifest.
        assert!(!is_storyboard("name = \"release\"\nentry = \"lint\"\n"));
        assert!(!is_storyboard("[package]\nname = \"x\"\n"));
        assert!(!is_storyboard("[demo]\noutput = \"d\"\n"));
        assert!(!is_storyboard("not toml ["));
    }

    #[test]
    #[cfg(unix)]
    fn a_folder_is_searched_and_a_file_is_taken_as_given() {
        let root = std::env::temp_dir().join(format!("matinee-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("b/demo.toml", STORY);
        write("a/tour.toml", STORY);
        write("a/release.toml", "name = \"release\"\n");
        write("Cargo.toml", "[package]\nname = \"x\"\n");
        write(".hidden/demo.toml", STORY);
        write("target/demo.toml", STORY);
        write("node_modules/x/demo.toml", STORY);
        std::os::unix::fs::symlink(root.join("b"), root.join("link")).unwrap();

        let found = storyboards(&root).unwrap();
        let names: Vec<String> = found
            .iter()
            .map(|p| p.strip_prefix(&root).unwrap().display().to_string())
            .collect();
        assert_eq!(names, ["a/tour.toml", "b/demo.toml"]);

        let file = root.join("a/release.toml");
        assert_eq!(storyboards(&file).unwrap(), [file]);
        assert!(storyboards(&root.join("missing")).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
