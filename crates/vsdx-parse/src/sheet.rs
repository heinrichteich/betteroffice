use crate::VsdxError;
use crate::xml::ParseBudget;
use crate::xml::{XmlElement, XmlNode};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    pub name: String,
    pub formula: Option<String>,
    pub value: Option<String>,
    pub unit: Option<String>,
    pub del: bool,
    pub other_attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub name: String,
    pub index: Option<i32>,
    pub del: bool,
    pub rows: Vec<Row>,
    pub other_attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub index: Option<i32>,
    pub name: Option<String>,
    pub row_type: Option<String>,
    pub del: bool,
    pub cells: Vec<Cell>,
    pub other_attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shape {
    pub id: i32,
    pub name: Option<String>,
    pub name_u: Option<String>,
    pub shape_type: Option<String>,
    pub master: Option<i32>,
    pub master_shape: Option<i32>,
    pub line_style: Option<i32>,
    pub fill_style: Option<i32>,
    pub text_style: Option<i32>,
    pub cells: Vec<Cell>,
    pub sections: Vec<Section>,
    pub text: Option<Vec<TextToken>>,
    pub children: Vec<Shape>,
    pub del: bool,
    pub other_attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TextToken {
    Literal(String),
    CharacterRun(i32),
    ParagraphRun(i32),
    Tab(i32),
    Field(i32),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connect {
    pub from_sheet: i32,
    pub from_cell: Option<String>,
    pub from_part: Option<i32>,
    pub to_sheet: i32,
    pub to_cell: Option<String>,
    pub to_part: Option<i32>,
    pub other_attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sheet {
    pub cells: Vec<Cell>,
    pub sections: Vec<Section>,
    pub shapes: Vec<Shape>,
    pub connects: Vec<Connect>,
    pub other_attrs: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XmlRecord {
    pub name: String,
    pub attributes: Vec<(String, String)>,
}

pub(crate) fn parse_records(parent: Option<&XmlElement>) -> Vec<XmlRecord> {
    parent
        .into_iter()
        .flat_map(elements)
        .map(|element| XmlRecord {
            name: element.local_name().to_owned(),
            attributes: element.attributes.clone(),
        })
        .collect()
}

pub(crate) fn parse_sheet(
    root: &XmlElement,
    part: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<Sheet, VsdxError> {
    let mut sheet = Sheet {
        other_attrs: root.attributes.clone(),
        ..Sheet::default()
    };
    for child in elements(root) {
        match child.local_name() {
            "Cell" => sheet.cells.push(parse_cell(child, part, budget)?),
            "Section" => sheet.sections.push(parse_section(child, part, budget)?),
            "Shapes" => {
                for shape in elements(child).filter(|element| element.local_name() == "Shape") {
                    sheet.shapes.push(parse_shape(shape, part, budget)?);
                }
            }
            "Connects" => {
                for connect in elements(child).filter(|element| element.local_name() == "Connect") {
                    sheet.connects.push(parse_connect(connect)?);
                }
            }
            _ => {}
        }
    }
    Ok(sheet)
}

fn parse_shape(
    element: &XmlElement,
    part: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<Shape, VsdxError> {
    budget.charge_shape(part)?;
    let mut shape = Shape {
        id: required_i32(element, "ID", part)?,
        name: attr(element, "Name"),
        name_u: attr(element, "NameU"),
        shape_type: attr(element, "Type"),
        master: optional_i32(element, "Master", part)?,
        master_shape: optional_i32(element, "MasterShape", part)?,
        line_style: optional_i32(element, "LineStyle", part)?,
        fill_style: optional_i32(element, "FillStyle", part)?,
        text_style: optional_i32(element, "TextStyle", part)?,
        cells: Vec::new(),
        sections: Vec::new(),
        text: None,
        children: Vec::new(),
        del: deleted(element),
        other_attrs: other(
            element,
            &[
                "ID",
                "Name",
                "NameU",
                "Type",
                "Master",
                "MasterShape",
                "LineStyle",
                "FillStyle",
                "TextStyle",
                "Del",
            ],
        ),
    };
    for child in elements(element) {
        match child.local_name() {
            "Cell" => shape.cells.push(parse_cell(child, part, budget)?),
            "Section" => shape.sections.push(parse_section(child, part, budget)?),
            "Text" => shape.text = Some(parse_text(child, part)?),
            "Shapes" => {
                for nested in elements(child).filter(|nested| nested.local_name() == "Shape") {
                    shape.children.push(parse_shape(nested, part, budget)?);
                }
            }
            _ => {}
        }
    }
    Ok(shape)
}
fn parse_cell(
    element: &XmlElement,
    part: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<Cell, VsdxError> {
    budget.charge_cell(part)?;
    Ok(Cell {
        name: required(element, "N", part)?,
        formula: attr(element, "F"),
        value: attr(element, "V"),
        unit: attr(element, "U"),
        del: deleted(element),
        other_attrs: other(element, &["N", "F", "V", "U", "Del"]),
    })
}
fn parse_section(
    element: &XmlElement,
    part: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<Section, VsdxError> {
    budget.charge_section(part)?;
    let mut section = Section {
        name: required(element, "N", part)?,
        index: optional_i32(element, "IX", part)?,
        del: deleted(element),
        rows: Vec::new(),
        other_attrs: other(element, &["N", "IX", "Del"]),
    };
    for row in elements(element).filter(|child| child.local_name() == "Row") {
        section.rows.push(parse_row(row, part, budget)?);
    }
    Ok(section)
}
fn parse_row(
    element: &XmlElement,
    part: &str,
    budget: &mut ParseBudget<'_>,
) -> Result<Row, VsdxError> {
    budget.charge_row(part)?;
    let mut row = Row {
        index: optional_i32(element, "IX", part)?,
        name: attr(element, "N"),
        row_type: attr(element, "T"),
        del: deleted(element),
        cells: Vec::new(),
        other_attrs: other(element, &["IX", "N", "T", "Del"]),
    };
    for cell in elements(element).filter(|child| child.local_name() == "Cell") {
        row.cells.push(parse_cell(cell, part, budget)?);
    }
    Ok(row)
}
fn parse_text(element: &XmlElement, part: &str) -> Result<Vec<TextToken>, VsdxError> {
    element
        .children
        .iter()
        .map(|node| match node {
            XmlNode::Text(text) => Ok(TextToken::Literal(text.clone())),
            XmlNode::Element(marker) => match marker.local_name() {
                "cp" => Ok(TextToken::CharacterRun(required_i32(marker, "IX", part)?)),
                "pp" => Ok(TextToken::ParagraphRun(required_i32(marker, "IX", part)?)),
                "tp" => Ok(TextToken::Tab(required_i32(marker, "IX", part)?)),
                "fld" => Ok(TextToken::Field(required_i32(marker, "IX", part)?)),
                name => Err(malformed(part, format!("unknown Text child {name}"))),
            },
        })
        .collect()
}
fn parse_connect(element: &XmlElement) -> Result<Connect, VsdxError> {
    Ok(Connect {
        from_sheet: required_i32(element, "FromSheet", "Connect")?,
        from_cell: attr(element, "FromCell"),
        from_part: optional_i32(element, "FromPart", "Connect")?,
        to_sheet: required_i32(element, "ToSheet", "Connect")?,
        to_cell: attr(element, "ToCell"),
        to_part: optional_i32(element, "ToPart", "Connect")?,
        other_attrs: other(
            element,
            &[
                "FromSheet",
                "FromCell",
                "FromPart",
                "ToSheet",
                "ToCell",
                "ToPart",
            ],
        ),
    })
}
fn elements(element: &XmlElement) -> impl Iterator<Item = &XmlElement> {
    element.children.iter().filter_map(|node| match node {
        XmlNode::Element(element) => Some(element),
        XmlNode::Text(_) => None,
    })
}
fn attr(element: &XmlElement, name: &str) -> Option<String> {
    element.attribute(name).map(str::to_owned)
}
fn required(element: &XmlElement, name: &str, part: &str) -> Result<String, VsdxError> {
    attr(element, name).ok_or_else(|| malformed(part, format!("missing {name}")))
}
fn required_i32(element: &XmlElement, name: &str, part: &str) -> Result<i32, VsdxError> {
    required(element, name, part)?
        .parse()
        .map_err(|_| malformed(part, format!("invalid {name}")))
}
fn optional_i32(element: &XmlElement, name: &str, part: &str) -> Result<Option<i32>, VsdxError> {
    attr(element, name)
        .map(|value| {
            value
                .parse()
                .map_err(|_| malformed(part, format!("invalid {name}")))
        })
        .transpose()
}
fn deleted(element: &XmlElement) -> bool {
    element.attribute("Del") == Some("1")
}
fn other(element: &XmlElement, known: &[&str]) -> Vec<(String, String)> {
    element
        .attributes
        .iter()
        .filter(|(name, _)| !known.contains(&name.as_str()))
        .cloned()
        .collect()
}
fn malformed(part: &str, message: String) -> VsdxError {
    VsdxError::MalformedXml {
        part: part.to_owned(),
        offset: 0,
        message,
    }
}
