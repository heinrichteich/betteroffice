use ooxml_drawingml::GeometryPathCommand;
use serde::{Deserialize, Serialize};

pub const CONTRACT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VsdxDisplayList {
    pub contract_version: u32,
    pub width: f32,
    pub height: f32,
    /// The only transform from Visio inches/Y-up into canvas pixels/Y-down.
    pub paint_transform: PaintTransform,
    pub primitives: Vec<Primitive>,
}

impl VsdxDisplayList {
    pub fn validate(&self) -> Result<(), String> {
        if self.contract_version != CONTRACT_VERSION {
            return Err(format!(
                "unsupported VSDX display-list contract version {}",
                self.contract_version
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaintTransform {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Paint {
    Solid { color: String },
    Gradient { stops: Vec<GradientStop> },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradientStop {
    pub position: f32,
    pub color: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stroke {
    pub color: String,
    pub width: f32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dashed: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transform {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rotation_deg: f32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_x: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub flip_y: bool,
}
impl Transform {
    pub fn is_identity(&self) -> bool {
        self.rotation_deg == 0.0 && !self.flip_x && !self.flip_y
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Primitive {
    Shape {
        id: String,
        z_order: u32,
        path: Vec<GeometryPathCommand>,
        fill: Option<Paint>,
        stroke: Option<Stroke>,
        #[serde(default, skip_serializing_if = "Transform::is_identity")]
        transform: Transform,
    },
    Image {
        id: String,
        z_order: u32,
        asset_id: String,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        #[serde(default, skip_serializing_if = "Transform::is_identity")]
        transform: Transform,
    },
    TextBox {
        id: String,
        z_order: u32,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        paragraphs: Vec<TextParagraph>,
        lines: Vec<PositionedLine>,
    },
    Placeholder {
        id: String,
        z_order: u32,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        reason: String,
    },
    Group {
        id: String,
        z_order: u32,
        primitives: Vec<Primitive>,
        #[serde(default, skip_serializing_if = "Transform::is_identity")]
        transform: Transform,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextParagraph {
    pub runs: Vec<TextRun>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRun {
    pub text: String,
    pub family: String,
    pub size_px: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionedLine {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub start: u32,
    pub end: u32,
    pub caret_stops: Vec<CaretStop>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaretStop {
    pub position: u32,
    pub x: f32,
}
fn is_false(value: &bool) -> bool {
    !*value
}
fn is_zero(value: &f32) -> bool {
    *value == 0.0
}
