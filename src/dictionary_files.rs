use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn dictionary_files(root: &Path) -> Vec<PathBuf> {
    let manifest = root.join("rime_ice.dict.yaml");
    if let Ok(contents) = fs::read_to_string(&manifest) {
        let imports = parse_imports(&contents);
        if !imports.is_empty() {
            let mut files = vec![manifest];
            for import in imports {
                if let Some(path) = resolve_import(root, &import) {
                    if !files.contains(&path) {
                        files.push(path);
                    }
                }
            }
            return files;
        }
    }

    let mut files = vec![manifest];
    if let Ok(entries) = fs::read_dir(root.join("cn_dicts")) {
        files.extend(
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("yaml")),
        );
    }
    files[1..].sort();
    files
}

fn parse_imports(contents: &str) -> Vec<String> {
    let mut in_import_tables = false;
    let mut imports = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed == "import_tables:" {
            in_import_tables = true;
            continue;
        }
        if in_import_tables && !line.starts_with(' ') && !line.starts_with('\t') {
            break;
        }
        if !in_import_tables {
            continue;
        }

        let Some(value) = trimmed.strip_prefix('-') else {
            continue;
        };
        let value = value.split('#').next().unwrap_or_default().trim();
        let value = value.trim_matches(|character| character == '\'' || character == '"');
        if !value.is_empty() {
            imports.push(value.to_string());
        }
    }
    imports
}

fn resolve_import(root: &Path, import: &str) -> Option<PathBuf> {
    let path = Path::new(import);
    let candidates = [
        root.join(path),
        root.join(format!("{import}.dict.yaml")),
        root.join(format!("{import}.yaml")),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::parse_imports;

    #[test]
    fn parses_only_enabled_import_tables() {
        let contents = r#"
import_tables:
  - cn_dicts/8105     # enabled
  # - cn_dicts/41448   # disabled
  - 'cn_dicts/base'

other:
  - should_not_be_loaded
"#;

        assert_eq!(
            parse_imports(contents),
            vec!["cn_dicts/8105", "cn_dicts/base"]
        );
    }
}
