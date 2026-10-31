use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FIXTURES: &[(&str, &[&str])] = &[
    ("spec/headings.md", &["§3.1", "§3.12", "§5"]),
    ("spec/paragraphs.md", &["§3.2", "§5", "§6"]),
    ("spec/inline.md", &["§3.3", "§3.4", "§3.13"]),
    ("spec/links.md", &["§3.5", "§5"]),
    ("spec/images.md", &["§3.6", "§9"]),
    ("spec/quotes.md", &["§3.7", "§4", "§5"]),
    ("spec/lists.md", &["§3.8", "§3.12", "§4", "§5"]),
    ("spec/code.md", &["§3.9", "§4", "§5"]),
    ("spec/overwide.md", &["§3.1", "§3.9", "§3.11", "§5"]),
    ("spec/hr.md", &["§3.10", "§4"]),
    ("spec/tables.md", &["§3.11", "§3.5", "§5", "§6"]),
    ("spec/spacing.md", &["§4"]),
    ("spec/footnotes.md", &["§3.13", "§9"]),
    ("spec/html.md", &["§3.13", "§9"]),
    ("links/anchors.md", &["§3.1", "§3.5"]),
    ("stream/large.md", &["§3.2", "§3.11", "§5"]),
    ("stream/hidden-blocks.md", &["§3.9", "§4", "§9"]),
    ("sidebar/many-headings.md", &["§3.1", "§5"]),
    ("sidebar/huge-setext.md", &["§3.1", "§5"]),
];

#[test]
fn fixtures_manifest_covers_required_files_and_readme() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let readme = fs::read_to_string(root.join("README.md")).expect("fixtures README must be UTF-8");
    assert!(readme.contains("P06"));

    let expected: HashSet<&str> = FIXTURES.iter().map(|(path, _)| *path).collect();
    assert_eq!(
        expected.len(),
        FIXTURES.len(),
        "duplicate expected fixture path"
    );

    let mut basenames = HashSet::new();
    for (path, sections) in FIXTURES {
        let file = root.join(path);
        let bytes =
            fs::read(&file).unwrap_or_else(|error| panic!("missing fixture {path}: {error}"));
        assert!(!bytes.is_empty(), "fixture {path} must not be empty");
        String::from_utf8(bytes)
            .unwrap_or_else(|error| panic!("fixture {path} must be UTF-8: {error}"));

        let basename = Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .expect("fixture basename must be UTF-8");
        assert!(
            basenames.insert(basename),
            "duplicate fixture basename {basename}"
        );

        let marker = format!("| `{path}` |");
        let mut rows = readme.lines().filter(|line| line.starts_with(&marker));
        let row = rows
            .next()
            .unwrap_or_else(|| panic!("README missing fixture row for {path}"));
        assert!(
            rows.next().is_none(),
            "README has duplicate fixture row for {path}"
        );
        let columns: Vec<&str> = row.split('|').map(str::trim).collect();
        assert!(
            columns.len() >= 5,
            "README row for {path} must have all columns"
        );
        for section in *sections {
            assert!(
                columns[2].contains(section),
                "README row for {path} must mention {section}"
            );
        }
        assert!(
            !columns[3].is_empty(),
            "README row for {path} must describe usage"
        );
        assert!(
            !columns[4].is_empty(),
            "README row for {path} must describe TUI/--once differences"
        );
    }

    let mut actual = Vec::new();
    collect_markdown(&root, &root, &mut actual).expect("fixtures directory must be readable");
    actual.sort();
    let mut expected_sorted: Vec<String> = expected.iter().map(|path| (*path).to_owned()).collect();
    expected_sorted.sort();
    assert_eq!(
        actual, expected_sorted,
        "fixture file set must exactly match the manifest"
    );
}

fn collect_markdown(root: &Path, directory: &Path, files: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_markdown(root, &path, files)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("md")
            && path.file_name().and_then(|name| name.to_str()) != Some("README.md")
        {
            let relative = path
                .strip_prefix(root)
                .expect("fixture must be under fixtures root")
                .to_string_lossy()
                .replace('\\', "/");
            files.push(relative);
        }
    }
    Ok(())
}
