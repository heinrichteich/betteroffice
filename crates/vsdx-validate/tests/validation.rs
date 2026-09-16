use vsdx_validate::{
    RULE_CONNECTOR_CROSSING, RULE_DANGLING_CONNECTOR, RULE_EMPTY_SHAPE_DATA,
    RULE_ISOLATED_SHAPE, RULE_OVERLAPPING_SHAPES, validate_package,
};

fn issue_keys() -> Vec<String> {
    let bytes = include_bytes!("../../vsdx-parse/tests/fixtures/validation.vsdx");
    let package = vsdx_parse::parse_vsdx(bytes).unwrap();
    let report = validate_package(&package);
    assert_eq!(report, validate_package(&package));
    report
        .issues
        .iter()
        .map(|issue| {
            format!(
                "{}:{}:{}:{}",
                issue.rule,
                issue.shape_id,
                issue.other_shape_id.unwrap_or(0),
                issue.endpoint.clone().or(issue.row.clone()).unwrap_or_default()
            )
        })
        .collect()
}

#[test]
fn validation_fixture_reports_each_default_rule_once() {
    assert_eq!(
        issue_keys(),
        [
            format!("{RULE_CONNECTOR_CROSSING}:6:7:"),
            format!("{RULE_DANGLING_CONNECTOR}:4:0:end"),
            format!("{RULE_DANGLING_CONNECTOR}:5:0:"),
            format!("{RULE_EMPTY_SHAPE_DATA}:1:0:Owner"),
            format!("{RULE_ISOLATED_SHAPE}:7:0:"),
            format!("{RULE_OVERLAPPING_SHAPES}:1:2:"),
        ]
    );
}

#[test]
fn pinned_corpus_validation_cost() {
    let Some(directory) = std::env::var_os("VSDX_CORPUS_DIR").map(std::path::PathBuf::from)
    else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(&directory) else {
        return;
    };
    let mut files: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("vsdx"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "pinned corpus has no vsdx files");
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let package = vsdx_parse::parse_vsdx(&bytes).unwrap();
        let started = std::time::Instant::now();
        let report = validate_package(&package);
        let elapsed = started.elapsed();
        let shapes: usize = package
            .page_contents
            .values()
            .map(|sheet| sheet.shapes().count())
            .sum();
        eprintln!(
            "validate {}: {} shapes, {} issues, {:?}",
            path.file_name().unwrap().to_string_lossy(),
            shapes,
            report.issues.len(),
            elapsed
        );
    }
}
