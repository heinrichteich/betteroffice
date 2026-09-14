use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use vsdx_eval::{
    DocumentReferences, Evaluation, Expr, PageShapeReferences, evaluate_cell_with_package_theme,
    evaluate_cell_with_shape_package_theme, parse as eval_parse,
};
use vsdx_parse::{
    Cell, ParseLimits, Row, Section, Shape, ShapeChild, ShapesChild, Sheet, VsdxError, parse_vsdx,
    write_vsdx,
};
use vsdx_render::{Primitive, Renderer};
use vsdx_resolve::{Lookup, ResolvedShape, Resolver};

const FONT_BYTES: &[u8] =
    include_bytes!("../../../ooxml-text/tests/fonts/LiberationSans-Regular.ttf");
const KNOWN_CELLS: [&str; 4] = ["PinX", "PinY", "Width", "Height"];
const VISIBILITY_CONTROLS: [&str; 3] = ["NoFill", "NoLine", "NoShow"];
const HISTOGRAM_CAP: usize = 20;

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileSurvey {
    name: String,
    parse_ok: bool,
    parse_error: Option<String>,
    part_count: usize,
    page_count: usize,
    roundtrip_identical: Option<bool>,
    shape_count: usize,
    master_shape_count: usize,
    resolve_absent: BTreeMap<String, usize>,
    evaluated: usize,
    unsupported_known: usize,
    unsupported_other: usize,
    error: usize,
    total: usize,
    unsupported_constructs: BTreeMap<String, usize>,
    unsupported_other_kinds: BTreeMap<String, usize>,
    error_kinds: BTreeMap<String, usize>,
    painted: usize,
    placeholders: usize,
    hidden: usize,
    placeholder_reasons: BTreeMap<String, usize>,
    render_page_errors: usize,
    geometry_rows: BTreeMap<String, usize>,
    master_geometry_rows: BTreeMap<String, usize>,
    visibility_carriers: BTreeMap<String, usize>,
    master_visibility_carriers: BTreeMap<String, usize>,
    visibility_placeholders: BTreeMap<String, usize>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Aggregate {
    files: usize,
    parse_ok: usize,
    parse_failed: usize,
    roundtrip_identical: usize,
    roundtrip_differ: usize,
    roundtrip_not_attempted: usize,
    pages: usize,
    shapes: usize,
    master_shapes: usize,
    resolve_absent: BTreeMap<String, usize>,
    evaluated: usize,
    unsupported_known: usize,
    unsupported_other: usize,
    error: usize,
    total: usize,
    unsupported_constructs: BTreeMap<String, usize>,
    unsupported_other_kinds: BTreeMap<String, usize>,
    error_kinds: BTreeMap<String, usize>,
    painted: usize,
    placeholders: usize,
    hidden: usize,
    placeholder_reasons: BTreeMap<String, usize>,
    render_page_errors: usize,
    geometry_rows: BTreeMap<String, usize>,
    master_geometry_rows: BTreeMap<String, usize>,
    visibility_carriers: BTreeMap<String, usize>,
    master_visibility_carriers: BTreeMap<String, usize>,
    visibility_placeholders: BTreeMap<String, usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExploreOutput {
    files: Vec<FileSurvey>,
    aggregate: Aggregate,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg != "--json-only") {
        return Err("usage: explore [--json-only]".into());
    }
    let json_only = !args.is_empty();
    let Some(directory) = std::env::var_os("VSDX_EXPLORE_DIR") else {
        eprintln!("VSDX_EXPLORE_DIR is unset; nothing to survey");
        return Ok(());
    };
    let directory = PathBuf::from(directory);
    let entries = fs::read_dir(&directory).map_err(|error| {
        format!(
            "cannot read VSDX_EXPLORE_DIR {}: {error}",
            directory.display()
        )
    })?;
    let mut paths = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
        if !is_file {
            continue;
        }
        let wanted = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("vsdx") || extension.eq_ignore_ascii_case("vstx")
            });
        if wanted {
            paths.push(path);
        }
    }
    paths.sort();
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        match fs::read(path) {
            Ok(bytes) => files.push(survey(&bytes, &name)),
            Err(error) => files.push(FileSurvey {
                name,
                parse_error: Some(error.kind().to_string()),
                ..Default::default()
            }),
        }
    }
    for file in &mut files {
        cap_file_histograms(file);
    }
    let mut aggregate = aggregate(&files);
    cap_aggregate_histograms(&mut aggregate);
    if !json_only {
        print_summary(&files, &aggregate);
    }
    println!(
        "{}",
        serde_json::to_string(&ExploreOutput { files, aggregate })?
    );
    Ok(())
}

fn survey(bytes: &[u8], name: &str) -> FileSurvey {
    let mut survey = FileSurvey {
        name: name.to_owned(),
        part_count: zip_part_count(bytes).unwrap_or(0),
        ..Default::default()
    };
    let package = match parse_vsdx(bytes) {
        Ok(package) => package,
        Err(error) => {
            survey.parse_error = Some(parse_error_kind(&error));
            return survey;
        }
    };
    survey.parse_ok = true;
    survey.part_count = zip_part_count(bytes).unwrap_or_else(|| logical_part_count(&package));
    survey.page_count = package.page_part_paths.len();
    survey.roundtrip_identical = Some(match write_vsdx(&package) {
        Ok(out) => parse_vsdx(&out).is_ok_and(|reparsed| reparsed == package),
        Err(_) => false,
    });
    survey.shape_count = package.page_contents.values().map(count_shapes).sum();
    survey.master_shape_count = package.master_contents.values().map(count_shapes).sum();
    survey.resolve_absent = resolve_absent(&package);
    let measurement = measure_formulas(&package);
    survey.evaluated = measurement.evaluated;
    survey.unsupported_known = measurement.unsupported_known;
    survey.unsupported_other = measurement.unsupported_other;
    survey.error = measurement.error;
    survey.total = measurement.total;
    survey.unsupported_constructs = measurement.unsupported_constructs;
    survey.unsupported_other_kinds = measurement.unsupported_other_kinds;
    survey.error_kinds = measurement.error_kinds;
    let render = render_pages(&package);
    survey.painted = render.painted;
    survey.placeholders = render.placeholders;
    survey.hidden = count_hidden(&package, &render.placeholder_ids);
    survey.placeholder_reasons = render.reasons;
    survey.render_page_errors = render.page_errors;
    let geometry = count_geometry(&package);
    survey.geometry_rows = geometry.page_rows;
    survey.master_geometry_rows = geometry.master_rows;
    let visibility = count_visibility(&package, &render.placeholder_ids);
    survey.visibility_carriers = visibility.page_carriers;
    survey.master_visibility_carriers = visibility.master_carriers;
    survey.visibility_placeholders = visibility.degraded;
    survey
}

/// Maps a parse failure to its error variant, dropping any embedded path or content.
fn parse_error_kind(error: &VsdxError) -> String {
    match error {
        VsdxError::Container(_) => "container",
        VsdxError::MissingPart(_) => "missing part",
        VsdxError::UnsupportedDocumentKind(_) => "unsupported document kind",
        VsdxError::ConflictingMainDocumentRelationships(_) => {
            "conflicting main document relationships"
        }
        VsdxError::MalformedXml { .. } => "malformed xml",
        VsdxError::UnsafeXml { .. } => "unsafe xml",
        VsdxError::ResourceLimit { .. } => "resource limit exceeded",
        VsdxError::InvalidRelationship { .. } => "invalid relationship",
        VsdxError::InvalidSpan => "invalid span",
        VsdxError::InvalidXmlCharacter => "invalid xml character",
        VsdxError::PatchLimit { .. } => "patch limit exceeded",
        VsdxError::InvalidCellEdit { .. } => "invalid cell edit",
    }
    .to_owned()
}

fn aggregate(files: &[FileSurvey]) -> Aggregate {
    let mut total = Aggregate {
        files: files.len(),
        ..Default::default()
    };
    for file in files {
        if file.parse_ok {
            total.parse_ok += 1;
        } else {
            total.parse_failed += 1;
        }
        match file.roundtrip_identical {
            Some(true) => total.roundtrip_identical += 1,
            Some(false) => total.roundtrip_differ += 1,
            None => total.roundtrip_not_attempted += 1,
        }
        total.pages += file.page_count;
        total.shapes += file.shape_count;
        total.master_shapes += file.master_shape_count;
        merge(&mut total.resolve_absent, &file.resolve_absent);
        total.evaluated += file.evaluated;
        total.unsupported_known += file.unsupported_known;
        total.unsupported_other += file.unsupported_other;
        total.error += file.error;
        total.total += file.total;
        merge(
            &mut total.unsupported_constructs,
            &file.unsupported_constructs,
        );
        merge(
            &mut total.unsupported_other_kinds,
            &file.unsupported_other_kinds,
        );
        merge(&mut total.error_kinds, &file.error_kinds);
        total.painted += file.painted;
        total.placeholders += file.placeholders;
        total.hidden += file.hidden;
        merge(&mut total.placeholder_reasons, &file.placeholder_reasons);
        total.render_page_errors += file.render_page_errors;
        merge(&mut total.geometry_rows, &file.geometry_rows);
        merge(&mut total.master_geometry_rows, &file.master_geometry_rows);
        merge(&mut total.visibility_carriers, &file.visibility_carriers);
        merge(
            &mut total.master_visibility_carriers,
            &file.master_visibility_carriers,
        );
        merge(
            &mut total.visibility_placeholders,
            &file.visibility_placeholders,
        );
    }
    total
}

fn merge(into: &mut BTreeMap<String, usize>, from: &BTreeMap<String, usize>) {
    for (key, value) in from {
        *into.entry(key.clone()).or_default() += value;
    }
}

fn cap_file_histograms(file: &mut FileSurvey) {
    cap_map(&mut file.resolve_absent);
    cap_map(&mut file.unsupported_constructs);
    cap_map(&mut file.unsupported_other_kinds);
    cap_map(&mut file.error_kinds);
    cap_map(&mut file.placeholder_reasons);
    cap_map(&mut file.geometry_rows);
    cap_map(&mut file.master_geometry_rows);
    cap_map(&mut file.visibility_carriers);
    cap_map(&mut file.master_visibility_carriers);
    cap_map(&mut file.visibility_placeholders);
}

fn cap_aggregate_histograms(total: &mut Aggregate) {
    cap_map(&mut total.resolve_absent);
    cap_map(&mut total.unsupported_constructs);
    cap_map(&mut total.unsupported_other_kinds);
    cap_map(&mut total.error_kinds);
    cap_map(&mut total.placeholder_reasons);
    cap_map(&mut total.geometry_rows);
    cap_map(&mut total.master_geometry_rows);
    cap_map(&mut total.visibility_carriers);
    cap_map(&mut total.master_visibility_carriers);
    cap_map(&mut total.visibility_placeholders);
}

fn cap_map(map: &mut BTreeMap<String, usize>) {
    if map.len() <= HISTOGRAM_CAP {
        return;
    }
    let keep = top(map, HISTOGRAM_CAP)
        .into_iter()
        .map(|(key, _)| key)
        .collect::<BTreeSet<_>>();
    map.retain(|key, _| keep.contains(key));
}

fn print_summary(files: &[FileSurvey], total: &Aggregate) {
    for file in files {
        let roundtrip = match file.roundtrip_identical {
            Some(true) => "identical",
            Some(false) => "differ",
            None => "not attempted",
        };
        eprintln!(
            "{} parse={} pages={} shapes={} roundtrip={} formulas={}/{} painted={} placeholders={} hidden={}",
            file.name,
            if file.parse_ok { "ok" } else { "error" },
            file.page_count,
            file.shape_count,
            roundtrip,
            file.evaluated,
            file.total,
            file.painted,
            file.placeholders,
            file.hidden,
        );
        if let Some(error) = &file.parse_error {
            eprintln!("  parse error: {error}");
        }
    }
    eprintln!(
        "aggregate files={} parse_ok={} parse_failed={} pages={} shapes={} masters={} roundtrip_identical={} roundtrip_differ={} roundtrip_not_attempted={}",
        total.files,
        total.parse_ok,
        total.parse_failed,
        total.pages,
        total.shapes,
        total.master_shapes,
        total.roundtrip_identical,
        total.roundtrip_differ,
        total.roundtrip_not_attempted,
    );
    eprintln!(
        "aggregate formulas evaluated={} unsupported_known={} unsupported_other={} error={} total={}",
        total.evaluated, total.unsupported_known, total.unsupported_other, total.error, total.total,
    );
    eprintln!(
        "aggregate render painted={} placeholders={} hidden={} page_errors={}",
        total.painted, total.placeholders, total.hidden, total.render_page_errors,
    );
    eprintln!(
        "aggregate geometry page NURBSTo={} SplineStart={} SplineKnot={} master NURBSTo={} SplineStart={} SplineKnot={}",
        row_count(&total.geometry_rows, "NURBSTo"),
        row_count(&total.geometry_rows, "SplineStart"),
        row_count(&total.geometry_rows, "SplineKnot"),
        row_count(&total.master_geometry_rows, "NURBSTo"),
        row_count(&total.master_geometry_rows, "SplineStart"),
        row_count(&total.master_geometry_rows, "SplineKnot"),
    );
    eprintln!(
        "aggregate visibility page_carriers={:?} master_carriers={:?} degraded={:?}",
        total.visibility_carriers, total.master_visibility_carriers, total.visibility_placeholders,
    );
    eprintln!(
        "aggregate resolve absent={:?}",
        top(&total.resolve_absent, 20)
    );
    eprintln!(
        "top unsupported constructs: {:?}",
        top(&total.unsupported_constructs, 20)
    );
    eprintln!(
        "top other unsupported kinds: {:?}",
        top(&total.unsupported_other_kinds, 20)
    );
    eprintln!("top error kinds: {:?}", top(&total.error_kinds, 20));
    eprintln!(
        "top placeholder reasons: {:?}",
        top(&total.placeholder_reasons, 20)
    );
    eprintln!("top geometry rows: {:?}", top(&total.geometry_rows, 20));
    eprintln!(
        "top master geometry rows: {:?}",
        top(&total.master_geometry_rows, 20)
    );
}

fn row_count(rows: &BTreeMap<String, usize>, row_type: &str) -> usize {
    rows.get(row_type).copied().unwrap_or(0)
}

fn top(map: &BTreeMap<String, usize>, limit: usize) -> Vec<(String, usize)> {
    let mut entries = map
        .iter()
        .map(|(key, value)| (key.clone(), *value))
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    entries.truncate(limit);
    entries
}

fn zip_part_count(bytes: &[u8]) -> Option<usize> {
    const EOCD: [u8; 4] = [0x50, 0x4b, 0x05, 0x06];
    const MIN_EOCD: usize = 22;
    if bytes.len() < MIN_EOCD {
        return None;
    }
    let start = bytes.len().saturating_sub(65_557 + MIN_EOCD);
    let mut index = bytes.len() - MIN_EOCD;
    loop {
        if bytes[index..].starts_with(&EOCD) {
            let total = u16::from_le_bytes([bytes[index + 10], bytes[index + 11]]) as usize;
            return Some(total);
        }
        if index == start {
            return None;
        }
        index -= 1;
    }
}

fn logical_part_count(package: &vsdx_parse::VsdxPackage) -> usize {
    let mut count = 1;
    if package.pages_part_path.is_some() {
        count += 1;
    }
    if package.masters_part_path.is_some() {
        count += 1;
    }
    if package.windows_part_path.is_some() {
        count += 1;
    }
    count
        + package.page_part_paths.len()
        + package.master_part_paths.len()
        + package.theme_part_paths.len()
}

fn count_shapes(sheet: &Sheet) -> usize {
    sheet.shapes().map(count_shape).sum()
}

fn count_shape(shape: &Shape) -> usize {
    1 + shape.shapes().map(count_shape).sum::<usize>()
}

fn resolve_absent(package: &vsdx_parse::VsdxPackage) -> BTreeMap<String, usize> {
    let mut absent = BTreeMap::new();
    let resolver = Resolver::new(package);
    for page in &package.page_part_paths {
        let Ok(shapes) = resolver.resolve_page_shapes(page) else {
            continue;
        };
        for shape in shapes.values() {
            for cell in KNOWN_CELLS {
                if matches!(shape.cell(cell), Some(Lookup::Absent)) {
                    *absent.entry(cell.to_owned()).or_default() += 1;
                }
            }
        }
    }
    absent
}

/// Counts page shapes that yield no primitive: deleted and NoShow subtrees plus the
/// unvisited children of shapes the renderer placeholdered without descending into.
fn count_hidden(package: &vsdx_parse::VsdxPackage, placeholder_ids: &BTreeSet<String>) -> usize {
    let resolver = Resolver::new(package);
    let mut hidden = 0;
    for page in &package.page_part_paths {
        let Ok(shapes) = resolver.resolve_page_shapes(page) else {
            continue;
        };
        let Some(sheet) = package.page_contents.get(page) else {
            continue;
        };
        for shape in sheet.shapes() {
            hidden += hidden_subtree(&shapes, &resolver, page, shape, placeholder_ids);
        }
    }
    hidden
}

fn hidden_subtree(
    shapes: &BTreeMap<u32, ResolvedShape>,
    resolver: &Resolver<'_>,
    page: &str,
    shape: &Shape,
    placeholder_ids: &BTreeSet<String>,
) -> usize {
    if shape_hidden(shapes, resolver, page, shape) {
        return count_shape(shape);
    }
    if placeholder_ids.contains(&format!("{page}:{}", shape.id)) {
        return shape.shapes().map(count_shape).sum();
    }
    shape
        .shapes()
        .map(|child| hidden_subtree(shapes, resolver, page, child, placeholder_ids))
        .sum()
}

fn shape_hidden(
    shapes: &BTreeMap<u32, ResolvedShape>,
    resolver: &Resolver<'_>,
    page: &str,
    shape: &Shape,
) -> bool {
    if let Some(resolved) = shapes.get(&shape.id) {
        return is_hidden(resolved);
    }
    resolver
        .resolve_shape(page, shape.id)
        .is_ok_and(|resolved| is_hidden(&resolved))
}

/// Mirrors the renderer's early-out for deleted shapes and nonzero NoShow.
fn is_hidden(resolved: &ResolvedShape) -> bool {
    if resolved.deleted {
        return true;
    }
    let Some(Lookup::Found(cell)) = resolved.cell("NoShow") else {
        return false;
    };
    cell.cell
        .value
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .is_some_and(|value| value.is_finite() && value != 0.0)
}

struct Measurement {
    evaluated: usize,
    unsupported_known: usize,
    unsupported_other: usize,
    error: usize,
    total: usize,
    unsupported_constructs: BTreeMap<String, usize>,
    unsupported_other_kinds: BTreeMap<String, usize>,
    error_kinds: BTreeMap<String, usize>,
}

fn measure_formulas(package: &vsdx_parse::VsdxPackage) -> Measurement {
    let mut measurement = Measurement {
        evaluated: 0,
        unsupported_known: 0,
        unsupported_other: 0,
        error: 0,
        total: 0,
        unsupported_constructs: BTreeMap::new(),
        unsupported_other_kinds: BTreeMap::new(),
        error_kinds: BTreeMap::new(),
    };
    let limits = ParseLimits::default();
    let resolver = Resolver::new(package);
    let document = package
        .document_sheet
        .as_ref()
        .and_then(|sheet| resolver.resolve_sheet(sheet).ok());
    for sheet in package
        .document_sheet
        .iter()
        .chain(package.style_sheets.iter())
        .chain(package.page_sheets.values())
        .chain(package.master_sheets.values())
    {
        let Ok(resolved) = resolver.resolve_sheet(sheet) else {
            continue;
        };
        let refs = DocumentReferences::new(&resolved, document.as_ref());
        for (name, cell) in sheet_formula_cells(sheet) {
            let Some(formula) = cell.formula.as_deref() else {
                continue;
            };
            let evaluation =
                evaluate_cell_with_package_theme(&name, formula, &refs, &limits, package);
            record_formula(formula, evaluation, &mut measurement);
        }
    }
    for (page, sheet) in &package.page_contents {
        let owned = sheet_references(sheet);
        let refs = DocumentReferences::new(&owned, document.as_ref());
        for (name, cell) in sheet_formula_cells(sheet) {
            let Some(formula) = cell.formula.as_deref() else {
                continue;
            };
            let evaluation =
                evaluate_cell_with_package_theme(&name, formula, &refs, &limits, package);
            record_formula(formula, evaluation, &mut measurement);
        }
        let Ok(page_refs) = PageShapeReferences::new(&resolver, page) else {
            continue;
        };
        for shape in shapes_in(sheet) {
            let refs = page_refs.for_shape(shape.id);
            let Some(resolved) = page_refs.shape(shape.id) else {
                continue;
            };
            for (name, cell) in shape_formula_cells(shape) {
                let Some(formula) = cell.formula.as_deref() else {
                    continue;
                };
                let evaluation = evaluate_cell_with_shape_package_theme(
                    &name, formula, &refs, &limits, resolved, package,
                );
                record_formula(formula, evaluation, &mut measurement);
            }
        }
    }
    for sheet in package.master_contents.values() {
        let owned = sheet_references(sheet);
        let refs = DocumentReferences::new(&owned, document.as_ref());
        for (name, cell) in sheet_formula_cells(sheet) {
            let Some(formula) = cell.formula.as_deref() else {
                continue;
            };
            let evaluation =
                evaluate_cell_with_package_theme(&name, formula, &refs, &limits, package);
            record_formula(formula, evaluation, &mut measurement);
        }
        for shape in shapes_in(sheet) {
            let Ok(resolved) = resolver.resolve_shape_in_sheet(shape, sheet) else {
                continue;
            };
            for (name, cell) in shape_formula_cells(shape) {
                let Some(formula) = cell.formula.as_deref() else {
                    continue;
                };
                let evaluation = evaluate_cell_with_shape_package_theme(
                    &name,
                    formula,
                    &DocumentReferences::new(&resolved, document.as_ref()),
                    &limits,
                    &resolved,
                    package,
                );
                record_formula(formula, evaluation, &mut measurement);
            }
        }
    }
    measurement
}

fn record_formula(formula: &str, evaluation: Evaluation, measurement: &mut Measurement) {
    measurement.total += 1;
    if let Ok(expression) = eval_parse(formula, &ParseLimits::default())
        && has_unsupported(&expression)
    {
        collect_unsupported(&expression, &mut measurement.unsupported_constructs);
    }
    match evaluation {
        Evaluation::Evaluated(_) => measurement.evaluated += 1,
        Evaluation::Unsupported(reason) => {
            if is_known_deferred_reason(&reason) {
                measurement.unsupported_known += 1;
            } else {
                measurement.unsupported_other += 1;
                *measurement
                    .unsupported_other_kinds
                    .entry(classify_unsupported(&reason))
                    .or_default() += 1;
            }
        }
        Evaluation::Error(error) => {
            measurement.error += 1;
            *measurement
                .error_kinds
                .entry(classify_error(&error.message))
                .or_default() += 1;
        }
    }
}

fn is_known_deferred_call(name: &str) -> bool {
    !matches!(
        name.to_ascii_uppercase().as_str(),
        "IF" | "AND"
            | "OR"
            | "NOT"
            | "MIN"
            | "MAX"
            | "ABS"
            | "INT"
            | "ROUND"
            | "CEILING"
            | "FLOOR"
            | "SQRT"
            | "SIN"
            | "COS"
            | "TAN"
            | "ATAN2"
            | "PI"
            | "MOD"
            | "SUM"
            | "TRUNC"
            | "SIGN"
            | "RGB"
            | "TINT"
            | "MSOTINT"
            | "SAT"
            | "THEMEVAL"
            | "THEMEGUARD"
            | "_XFTRIGGER"
            | "GUARD"
    )
}

fn is_known_deferred_reason(reason: &str) -> bool {
    if reason == "Inh has no concrete inherited value" {
        return true;
    }
    let name = reason
        .strip_prefix("unsupported function ")
        .or_else(|| reason.strip_suffix(" is not implemented"))
        .or_else(|| reason.strip_suffix(" is outside the phase-4 evaluator"));
    name.is_some_and(is_known_deferred_call)
}

fn has_unsupported(expression: &Expr) -> bool {
    match expression {
        Expr::Call(name, args) => is_known_deferred_call(name) || args.iter().any(has_unsupported),
        Expr::Unary(value) => has_unsupported(value),
        Expr::Binary(left, _, right) => has_unsupported(left) || has_unsupported(right),
        _ => false,
    }
}

fn collect_unsupported(expression: &Expr, counts: &mut BTreeMap<String, usize>) {
    match expression {
        Expr::Call(name, args) => {
            if has_unsupported(&Expr::Call(name.clone(), Vec::new())) {
                *counts.entry(fold_call_name(name)).or_default() += 1;
            }
            for argument in args {
                collect_unsupported(argument, counts);
            }
        }
        Expr::Unary(value) => collect_unsupported(value, counts),
        Expr::Binary(left, _, right) => {
            collect_unsupported(left, counts);
            collect_unsupported(right, counts);
        }
        _ => {}
    }
}

/// Folds cross-sheet call scopes into one bucket so per-shape references cannot fan out the histogram.
fn fold_call_name(name: &str) -> String {
    let upper = name.to_ascii_uppercase();
    let is_cross_sheet = upper.contains('!')
        && upper.split('!').next().is_some_and(|scope| {
            scope == "THEDOC"
                || scope == "THEPAGE"
                || (scope.starts_with("SHEET.")
                    && scope[6..].chars().all(|cell| cell.is_ascii_digit()))
        });
    if is_cross_sheet {
        return "<sheet-ref>".to_owned();
    }
    upper
}

/// Reduces an evaluator message to its error class, dropping any document-derived tail.
fn classify_error(message: &str) -> String {
    if let Some(name) = message.strip_prefix("unresolved reference ") {
        if name.starts_with("Sheet.")
            || matches!(name.split_once('!'), Some(("ThePage" | "TheDoc", _)))
        {
            return "unresolved cross-sheet reference".to_owned();
        }
        return "unresolved cell reference".to_owned();
    }
    if message == "colour used where a numeric value is required"
        || message == "numeric value used where a colour is required"
    {
        return "type error".to_owned();
    }
    if message == "missing argument" || message.contains(" requires ") {
        return "arity error".to_owned();
    }
    if message.contains("unit")
        || message.contains("dimensional")
        || message.contains("trigonometric argument")
    {
        return "unit/dimension error".to_owned();
    }
    if message.contains("limit exceeded") {
        return "budget/depth/step exceeded".to_owned();
    }
    "other".to_owned()
}

/// Reduces an evaluator unsupported reason to a bounded key, keeping only the function name tail.
fn classify_unsupported(reason: &str) -> String {
    if is_static_unsupported_reason(reason) {
        return reason.to_owned();
    }
    if let Some(name) = reason.strip_prefix("unsupported function ") {
        return format!("unsupported function {}", fold_call_name(name));
    }
    if let Some(name) = reason.strip_suffix(" is not implemented") {
        return format!("not implemented: {}", fold_call_name(name));
    }
    if let Some(name) = reason.strip_suffix(" is outside the phase-4 evaluator") {
        return format!("outside phase-4 evaluator: {}", fold_call_name(name));
    }
    "other unsupported".to_owned()
}

fn is_static_unsupported_reason(reason: &str) -> bool {
    matches!(
        reason,
        "Inh has no concrete inherited value"
            | "event cell is outside the display evaluation profile"
            | "TheText requires phase-4b text layout"
            | "string values are not display numbers"
            | "SQRT of dimensional values is not implemented"
            | "THEMEVAL colour-scheme index must be 1 through 8"
            | "THEMEVAL requires a string or integer theme value"
            | "THEMEVAL host-cell lookup requires theme-cell context"
            | "THEMEVAL has no resolvable theme"
            | "unresolvable THEMEVAL value"
            | "cell value is not a supported display literal"
            | "cell value has an unsupported unit"
            | "non-finite result"
            | "SETATREFEXPR/SETATREFEVAL transformations are not implemented"
            | "SETATREF set_expression handling is not implemented"
            | "SETATREF requires a cell-reference first argument"
    )
}

fn sheet_formula_cells(sheet: &Sheet) -> Vec<(String, &Cell)> {
    let mut values = sheet
        .cells()
        .filter(|cell| cell.formula.is_some())
        .map(|cell| (cell.name.clone(), cell))
        .collect::<Vec<_>>();
    for section in sheet.sections() {
        for row in section.rows() {
            values.extend(
                row.cells()
                    .filter(|cell| cell.formula.is_some())
                    .map(|cell| (section_cell_name(section, row, cell), cell)),
            );
        }
    }
    values
}

fn shape_formula_cells(shape: &Shape) -> Vec<(String, &Cell)> {
    let mut values = shape
        .cells()
        .filter(|cell| cell.formula.is_some())
        .map(|cell| (cell.name.clone(), cell))
        .collect::<Vec<_>>();
    for section in shape.sections() {
        for row in section.rows() {
            values.extend(
                row.cells()
                    .filter(|cell| cell.formula.is_some())
                    .map(|cell| (section_cell_name(section, row, cell), cell)),
            );
        }
    }
    values
}

fn section_cell_name(section: &Section, row: &Row, cell: &Cell) -> String {
    row.name.as_ref().map_or_else(
        || format!("{}.{}", section.name, cell.name),
        |row| format!("{}.{}.{}", section.name, row, cell.name),
    )
}

fn shapes_in(sheet: &Sheet) -> Vec<&Shape> {
    let mut values = Vec::new();
    for shape in sheet.shapes() {
        collect_shapes(shape, &mut values);
    }
    values
}

fn collect_shapes<'a>(shape: &'a Shape, values: &mut Vec<&'a Shape>) {
    values.push(shape);
    for child in &shape.children {
        if let ShapeChild::Shapes(children) = child {
            for child in children {
                if let ShapesChild::Shape(shape) = child {
                    collect_shapes(shape, values);
                }
            }
        }
    }
}

fn sheet_references(sheet: &Sheet) -> BTreeMap<String, String> {
    let mut refs = references(sheet.cells().map(|cell| (cell.name.clone(), cell)));
    for section in sheet.sections() {
        for row in section.rows() {
            for cell in row.cells() {
                refs.extend(references(std::iter::once((
                    section_cell_name(section, row, cell),
                    cell,
                ))));
            }
        }
    }
    refs
}

fn references<'a>(cells: impl Iterator<Item = (String, &'a Cell)>) -> BTreeMap<String, String> {
    cells
        .filter_map(|(name, cell)| cell.formula.as_ref().map(|formula| (name, formula.clone())))
        .collect()
}

struct RenderCounts {
    painted: usize,
    placeholders: usize,
    reasons: BTreeMap<String, usize>,
    placeholder_ids: BTreeSet<String>,
    page_errors: usize,
}

fn render_pages(package: &vsdx_parse::VsdxPackage) -> RenderCounts {
    let mut counts = RenderCounts {
        painted: 0,
        placeholders: 0,
        reasons: BTreeMap::new(),
        placeholder_ids: BTreeSet::new(),
        page_errors: 0,
    };
    let mut renderer = Renderer::default();
    if renderer
        .register_font("sans-serif", false, false, FONT_BYTES.to_vec())
        .is_err()
    {
        counts.page_errors = package.page_part_paths.len();
        return counts;
    }
    for page in &package.page_part_paths {
        match renderer.layout_page(package, page) {
            Ok(list) => count_primitives(
                &list.primitives,
                &mut counts.painted,
                &mut counts.placeholders,
                &mut counts.reasons,
                &mut counts.placeholder_ids,
            ),
            Err(_) => {
                counts.page_errors += 1;
            }
        }
    }
    counts
}

fn count_primitives(
    primitives: &[Primitive],
    painted: &mut usize,
    placeholders: &mut usize,
    reasons: &mut BTreeMap<String, usize>,
    ids: &mut BTreeSet<String>,
) {
    for primitive in primitives {
        match primitive {
            Primitive::Shape { .. } | Primitive::Image { .. } => {
                *painted += 1;
            }
            Primitive::Placeholder { id, reason, .. } => {
                *placeholders += 1;
                *reasons
                    .entry(classify_placeholder_reason(reason))
                    .or_default() += 1;
                ids.insert(id.clone());
            }
            Primitive::Group { primitives, .. } => {
                *painted += 1;
                count_primitives(primitives, painted, placeholders, reasons, ids);
            }
            Primitive::TextBox { .. } => {}
        }
    }
}

/// Reduces a renderer placeholder reason to its class, keeping only fixed-vocabulary tails.
fn classify_placeholder_reason(reason: &str) -> String {
    if let Some(detail) = reason.strip_prefix("unsupported Geometry section controls") {
        return section_control_class(detail);
    }
    if let Some(detail) = reason.strip_prefix("unsupported geometry") {
        let rows = geometry_issue_rows(detail);
        if rows.is_empty() {
            return "unsupported geometry".to_owned();
        }
        return format!("unsupported geometry: {}", rows.join(", "));
    }
    if reason.starts_with("unresolvable colour") {
        return "unresolvable colour".to_owned();
    }
    if reason.starts_with("connector route cannot be computed") {
        return "connector route cannot be computed".to_owned();
    }
    if matches!(
        reason,
        "group nesting depth exceeded"
            | "unresolvable transform"
            | "overflowing transform"
            | "dangling ForeignData image target"
            | "unsupported ForeignData image"
            | "shape has no Geometry section"
            | "1D shape is missing connector state"
            | "non-finite stroke width"
    ) {
        return reason.to_owned();
    }
    if reason.starts_with("missing colour")
        || reason.starts_with("missing Color")
        || reason.starts_with("unavailable colour")
        || reason.contains("palette index")
    {
        return "unresolvable colour".to_owned();
    }
    let error_class = classify_error(reason);
    if error_class != "other" {
        return format!("unresolvable colour: {error_class}");
    }
    let unsupported_class = classify_unsupported(reason);
    if unsupported_class != "other unsupported" {
        return format!("unresolvable colour: {unsupported_class}");
    }
    "other placeholder".to_owned()
}

/// Keeps which of the fixed NoFill/NoLine/NoShow controls fired, dropping the section index.
fn section_control_class(detail: &str) -> String {
    let mut controls = VISIBILITY_CONTROLS
        .into_iter()
        .filter(|control| detail.contains(*control))
        .collect::<Vec<_>>();
    controls.sort_unstable();
    if controls.is_empty() {
        return "unsupported Geometry section controls".to_owned();
    }
    format!(
        "unsupported Geometry section controls: {}",
        controls.join(", ")
    )
}

/// Extracts geometry-issue row types while dropping cell names and other Debug detail.
fn geometry_issue_rows(detail: &str) -> Vec<String> {
    let mut rows = BTreeSet::new();
    for marker in [
        "row_type: \"",
        "UnsupportedRowType(\"",
        "UnsupportedSectionControl(\"",
    ] {
        let mut rest = detail;
        while let Some(found) = rest.find(marker) {
            rest = &rest[found + marker.len()..];
            let end = rest.find('"').unwrap_or(rest.len());
            let name = &rest[..end];
            if !name.is_empty()
                && name.len() <= 64
                && name
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
            {
                rows.insert(name.to_owned());
            }
        }
    }
    rows.into_iter().collect()
}

struct GeometryCounts {
    page_rows: BTreeMap<String, usize>,
    master_rows: BTreeMap<String, usize>,
}

fn count_geometry(package: &vsdx_parse::VsdxPackage) -> GeometryCounts {
    let mut counts = GeometryCounts {
        page_rows: BTreeMap::new(),
        master_rows: BTreeMap::new(),
    };
    for sheet in package.page_contents.values() {
        count_geometry_rows(sheet, &mut counts.page_rows);
    }
    for sheet in package.master_contents.values() {
        count_geometry_rows(sheet, &mut counts.master_rows);
    }
    counts
}

fn count_geometry_rows(sheet: &Sheet, rows: &mut BTreeMap<String, usize>) {
    for shape in shapes_in(sheet) {
        for section in shape.sections() {
            if section.name != "Geometry" {
                continue;
            }
            for row in section.rows() {
                if row.del {
                    continue;
                }
                *rows
                    .entry(row.row_type.clone().unwrap_or_else(|| "(none)".to_owned()))
                    .or_default() += 1;
            }
        }
    }
}

struct VisibilityCounts {
    page_carriers: BTreeMap<String, usize>,
    master_carriers: BTreeMap<String, usize>,
    degraded: BTreeMap<String, usize>,
}

fn count_visibility(
    package: &vsdx_parse::VsdxPackage,
    placeholder_ids: &BTreeSet<String>,
) -> VisibilityCounts {
    let mut counts = VisibilityCounts {
        page_carriers: BTreeMap::new(),
        master_carriers: BTreeMap::new(),
        degraded: BTreeMap::new(),
    };
    let resolver = Resolver::new(package);
    for page in &package.page_part_paths {
        let Ok(shapes) = resolver.resolve_page_shapes(page) else {
            continue;
        };
        for (id, resolved) in &shapes {
            let controls = section_controls(resolved);
            for control in &controls {
                *counts.page_carriers.entry(control.clone()).or_default() += 1;
            }
            if placeholder_ids.contains(&format!("{page}:{id}")) {
                for control in &controls {
                    *counts.degraded.entry(control.clone()).or_default() += 1;
                }
            }
        }
    }
    for sheet in package.master_contents.values() {
        for shape in shapes_in(sheet) {
            let Ok(resolved) = resolver.resolve_shape_in_sheet(shape, sheet) else {
                continue;
            };
            for control in section_controls(&resolved) {
                *counts.master_carriers.entry(control).or_default() += 1;
            }
        }
    }
    counts
}

/// Reads Geometry section controls from the resolved shape, after master inheritance.
fn section_controls(resolved: &ResolvedShape) -> Vec<String> {
    let mut controls = Vec::new();
    for control in VISIBILITY_CONTROLS {
        let carries = resolved.sections.values().any(|section| {
            section.name == "Geometry"
                && !section.deleted
                && section
                    .unsupported_controls
                    .iter()
                    .any(|name| name == control)
        });
        if carries {
            controls.push(control.to_owned());
        }
    }
    controls
}

#[cfg(test)]
mod tests {
    use super::{measure_formulas, survey};

    #[test]
    fn foundation_fixture_survey_is_consistent() {
        let bytes = include_bytes!("../../../vsdx-parse/tests/fixtures/foundation.vsdx");
        let file = survey(bytes, "foundation.vsdx");
        assert!(file.parse_ok);
        assert_eq!(file.roundtrip_identical, Some(true));
        assert_eq!(
            file.evaluated + file.unsupported_known + file.unsupported_other + file.error,
            file.total
        );
        assert_eq!(
            file.painted + file.placeholders + file.hidden,
            file.shape_count
        );
    }

    #[test]
    fn harness_formula_totals_match_eval_pinned_corpus() {
        let Some(directory) = std::env::var_os("VSDX_CORPUS_DIR") else {
            eprintln!("SKIPPED HARNESS FORMULA PIN: VSDX_CORPUS_DIR is unset");
            return;
        };
        let directory = std::path::PathBuf::from(directory);
        let mut evaluated = 0;
        let mut unsupported_known = 0;
        let mut unsupported_other = 0;
        let mut error = 0;
        let mut total = 0;
        for file in ["lichtsysteme.vsdx", "soundplan.vsdx"] {
            let bytes = std::fs::read(directory.join(file)).expect("read corpus file");
            let package = vsdx_parse::parse_vsdx(&bytes).expect("parse corpus package");
            let measurement = measure_formulas(&package);
            evaluated += measurement.evaluated;
            unsupported_known += measurement.unsupported_known;
            unsupported_other += measurement.unsupported_other;
            error += measurement.error;
            total += measurement.total;
        }
        assert_eq!(
            (
                evaluated,
                unsupported_known,
                unsupported_other,
                error,
                total
            ),
            (3679, 1892, 1256, 165, 6992)
        );
    }
}
