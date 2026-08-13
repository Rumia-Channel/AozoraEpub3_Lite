use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use zip::ZipArchive;

#[test]
#[ignore = "requires generated Java and Rust fixture directories"]
fn compares_generated_epubs_semantically() {
    let java_dir = required_directory("AOZORA_JAVA_DIR");
    let rust_dir = required_directory("AOZORA_RUST_DIR");
    let java_files = epub_files(&java_dir);
    let rust_files = epub_files(&rust_dir);
    assert_eq!(
        java_files, rust_files,
        "generated EPUB file sets differ: Java={java_files:?}, Rust={rust_files:?}"
    );

    for file_name in java_files {
        let java_entries = read_entries(&java_dir.join(&file_name));
        let rust_entries = read_entries(&rust_dir.join(&file_name));
        assert_eq!(
            java_entries.keys().collect::<Vec<_>>(),
            rust_entries.keys().collect::<Vec<_>>(),
            "{}: EPUB entry sets differ",
            file_name
        );
        for (entry_name, java_content) in &java_entries {
            let rust_content = rust_entries
                .get(entry_name)
                .unwrap_or_else(|| unreachable!("entry keys were compared above"));
            assert_eq!(
                canonical_entry(entry_name, java_content),
                canonical_entry(entry_name, rust_content),
                "{file_name}: entry {entry_name} differs"
            );
        }
    }
}

fn required_directory(name: &str) -> PathBuf {
    let value = env::var_os(name).unwrap_or_else(|| {
        panic!("{name} must point to a generated EPUB directory when this test is enabled")
    });
    let path = PathBuf::from(value);
    assert!(
        path.is_dir(),
        "{name} is not a directory: {}",
        path.display()
    );
    path
}

fn epub_files(directory: &Path) -> Vec<String> {
    let mut files = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| {
            let path = entry.unwrap().path();
            assert!(
                path.extension()
                    .is_some_and(|extension| extension == "epub"),
                "unexpected non-EPUB file in {}: {}",
                directory.display(),
                path.display()
            );
            path.file_name().unwrap().to_string_lossy().into_owned()
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn read_entries(path: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    let file =
        File::open(path).unwrap_or_else(|error| panic!("cannot open {}: {error}", path.display()));
    let mut archive = ZipArchive::new(file)
        .unwrap_or_else(|error| panic!("invalid EPUB {}: {error}", path.display()));
    let mut entries = std::collections::BTreeMap::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        let name = entry.name().to_owned();
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        assert!(
            entries.insert(name.clone(), content).is_none(),
            "duplicate EPUB entry: {name}"
        );
    }
    entries
}

fn canonical_entry(name: &str, content: &[u8]) -> Vec<u8> {
    if !is_text_entry(name) {
        return content.to_vec();
    }
    let text = String::from_utf8_lossy(content)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let text = if name.ends_with("standard.opf") {
        replace_modified_timestamp(&text)
    } else {
        text
    };
    text.into_bytes()
}

fn is_text_entry(name: &str) -> bool {
    name.ends_with(".css")
        || name.ends_with(".ncx")
        || name.ends_with(".opf")
        || name.ends_with(".svg")
        || name.ends_with(".xhtml")
        || name.ends_with(".xml")
}

fn replace_modified_timestamp(text: &str) -> String {
    let marker = "<meta property=\"dcterms:modified\">";
    let Some(start) = text.find(marker) else {
        return text.to_owned();
    };
    let content_start = start + marker.len();
    let Some(relative_end) = text[content_start..].find("</meta>") else {
        return text.to_owned();
    };
    let content_end = content_start + relative_end;
    let mut output = String::with_capacity(text.len());
    output.push_str(&text[..content_start]);
    output.push_str("DATE");
    output.push_str(&text[content_end..]);
    output
}
