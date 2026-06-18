use std::fs;
use std::path::Path;

const EXPECTED_FILE_COUNT: usize = 2_385;
const EXPECTED_INPUT_COUNT: usize = 339;
const MANIFEST: &str = include_str!("lld/ELF.MANIFEST");

#[test]
fn lld_elf_reference_corpus_is_complete() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lld/ELF");
    let expected = manifest_paths();
    let actual = corpus_paths(&root);

    assert_eq!(expected.len(), EXPECTED_FILE_COUNT);
    assert_eq!(actual.len(), EXPECTED_FILE_COUNT);
    assert_eq!(
        expected
            .iter()
            .filter(|path| path.starts_with("Inputs/") || path.contains("/Inputs/"))
            .count(),
        EXPECTED_INPUT_COUNT
    );
    assert_eq!(expected, actual);
}

#[test]
fn lld_elf_reference_corpus_contains_high_risk_linker_cases() {
    let expected = manifest_paths();
    for path in [
        "relro.s",
        "x86-64-tls-ie.s",
        "x86-64-tls-gdie.s",
        "x86-64-tlsdesc-gd.s",
        "x86-64-tlsdesc-ld.s",
        "x86-64-tls-dynamic.s",
        "x86-64-plt.s",
        "x86-64-rela.s",
        "relocation-copy.s",
        "gnu-ifunc-plt.s",
    ] {
        assert!(
            expected.binary_search(&path.to_owned()).is_ok(),
            "missing {path}"
        );
    }
}

fn manifest_paths() -> Vec<String> {
    MANIFEST
        .lines()
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn corpus_paths(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    collect_corpus_paths(root, root, &mut paths);
    paths.sort();
    paths
}

fn collect_corpus_paths(root: &Path, dir: &Path, paths: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read lld corpus directory") {
        let entry = entry.expect("read lld corpus entry");
        let path = entry.path();
        if path.is_dir() {
            collect_corpus_paths(root, &path, paths);
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .expect("corpus path should be under root")
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        paths.push(rel);
    }
}
