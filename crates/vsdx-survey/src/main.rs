use std::collections::BTreeMap;
use std::env;
use std::fs;

use betteroffice_vsdx::Diagram;
use serde::Serialize;
use vsdx_eval::{Evaluation, PageShapeReferences, evaluate_cell};
use vsdx_parse::ParseLimits;
use vsdx_render::{DiagnosticCategory, Primitive, Renderer};
use vsdx_resolve::{Lookup, Resolver};

/// One diagram surveyed by the office-quality harness.
#[derive(Serialize)]
struct FileSurvey {
    file: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    pages: usize,
    shapes: usize,
    primitives: usize,
    placeholders: usize,
    evaluated: u64,
    unsupported: u64,
    eval_errors: u64,
    fidelity: u64,
    integrity: u64,
    diagnostics: BTreeMap<String, u64>,
}

/// Whole survey run written to the coverage artifact.
#[derive(Serialize)]
struct SurveyOutput {
    schema_version: u32,
    files: Vec<FileSurvey>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let inputs = env::args()
        .skip(1)
        .map(|argument| {
            argument
                .split_once('=')
                .map(|(name, path)| (name.to_owned(), path.to_owned()))
                .ok_or_else(|| format!("survey input must be NAME=PATH: {argument}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if inputs.is_empty() {
        return Err("usage: betteroffice-vsdx-survey NAME=PATH [...]".into());
    }
    let mut files = Vec::with_capacity(inputs.len());
    for (name, path) in inputs {
        files.push(survey_file(&name, &path));
    }
    println!(
        "{}",
        serde_json::to_string(&SurveyOutput {
            schema_version: 1,
            files
        })?
    );
    Ok(())
}

fn survey_file(name: &str, path: &str) -> FileSurvey {
    match survey_path(path) {
        Ok(mut survey) => {
            survey.file = name.to_owned();
            survey
        }
        Err(error) => FileSurvey {
            file: name.to_owned(),
            status: "failed".into(),
            error: Some(error),
            pages: 0,
            shapes: 0,
            primitives: 0,
            placeholders: 0,
            evaluated: 0,
            unsupported: 0,
            eval_errors: 0,
            fidelity: 0,
            integrity: 0,
            diagnostics: BTreeMap::new(),
        },
    }
}

fn survey_path(path: &str) -> Result<FileSurvey, String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let diagram = Diagram::open(&bytes).map_err(|error| format!("{error:?}"))?;
    let package = diagram.package();
    let resolver = Resolver::new(package);
    let renderer = Renderer::default();
    let mut histogram: BTreeMap<String, u64> = BTreeMap::new();
    let mut fidelity = 0u64;
    let mut integrity = 0u64;
    let mut evaluated = 0u64;
    let mut unsupported = 0u64;
    let mut eval_errors = 0u64;
    let mut shapes = 0usize;
    let mut primitives = 0usize;
    let mut placeholders = 0usize;
    for page in &package.page_part_paths {
        let references =
            PageShapeReferences::new(&resolver, page).map_err(|error| error.to_string())?;
        shapes += references.shapes().len();
        for (shape_id, shape) in references.shapes() {
            let scoped = references.for_shape(*shape_id);
            for (cell_name, lookup) in &shape.cells {
                let Lookup::Found(cell) = lookup else {
                    continue;
                };
                let Some(expression) = cell.cell.formula.as_deref().or(cell.cell.value.as_deref())
                else {
                    continue;
                };
                match evaluate_cell(cell_name, expression, &scoped, &ParseLimits::default()) {
                    Evaluation::Evaluated(_) => evaluated += 1,
                    Evaluation::Unsupported(_) => unsupported += 1,
                    Evaluation::Error(_) => eval_errors += 1,
                }
            }
        }
        let list = renderer
            .layout_page(package, page)
            .map_err(|error| error.to_string())?;
        count_primitives(
            &list.primitives,
            &mut primitives,
            &mut placeholders,
            &mut histogram,
        );
    }
    for (code, count) in &histogram {
        match DiagnosticCategory::for_code(code) {
            DiagnosticCategory::Fidelity => fidelity += count,
            DiagnosticCategory::Integrity => integrity += count,
        }
    }
    Ok(FileSurvey {
        file: String::new(),
        status: "ok".into(),
        error: None,
        pages: package.page_part_paths.len(),
        shapes,
        primitives,
        placeholders,
        evaluated,
        unsupported,
        eval_errors,
        fidelity,
        integrity,
        diagnostics: histogram,
    })
}

fn count_primitives(
    items: &[Primitive],
    primitives: &mut usize,
    placeholders: &mut usize,
    histogram: &mut BTreeMap<String, u64>,
) {
    for item in items {
        match item {
            Primitive::Shape { diagnostics, .. } => {
                *primitives += 1;
                for diagnostic in diagnostics {
                    *histogram.entry(diagnostic.code.clone()).or_insert(0) += 1;
                }
            }
            Primitive::TextBox { paragraphs, .. } => {
                *primitives += 1;
                for paragraph in paragraphs {
                    for run in &paragraph.runs {
                        for diagnostic in &run.diagnostics {
                            *histogram.entry(diagnostic.code.clone()).or_insert(0) += 1;
                        }
                    }
                }
            }
            Primitive::Image { .. } => {
                *primitives += 1;
            }
            Primitive::Placeholder { .. } => {
                *primitives += 1;
                *placeholders += 1;
            }
            Primitive::Group {
                primitives: inner, ..
            } => {
                count_primitives(inner, primitives, placeholders, histogram);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let root = env!("CARGO_MANIFEST_DIR");
        format!("{root}/../vsdx-parse/tests/fixtures/{name}")
    }

    #[test]
    fn survey_counts_a_committed_fixture() {
        let survey = survey_path(&fixture("foundation.vsdx")).expect("fixture surveys");
        assert_eq!(survey.status, "ok");
        assert!(survey.pages >= 1);
        assert!(survey.primitives >= 1);
        assert_eq!(
            survey.fidelity + survey.integrity,
            survey.diagnostics.values().sum::<u64>()
        );
    }

    #[test]
    fn survey_histogram_keeps_every_code() {
        let survey = survey_path(&fixture("guard-format.vsdx")).expect("fixture surveys");
        let total: u64 = survey.diagnostics.values().sum::<u64>();
        assert_eq!(total, survey.fidelity + survey.integrity);
        for code in survey.diagnostics.keys() {
            assert!(!code.is_empty());
        }
    }

    #[test]
    fn survey_reports_a_missing_file_without_aborting() {
        let survey = survey_file("missing", "/definitely/not/a/diagram.vsdx");
        assert_eq!(survey.status, "failed");
        assert!(!survey.error.unwrap_or_default().is_empty());
    }
}
