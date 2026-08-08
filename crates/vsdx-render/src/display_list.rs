use ooxml_drawingml::GeometryPathCommand;
use serde::{Deserialize, Serialize};

pub const CONTRACT_VERSION: u32 = 1;

/// Replay primitives in ascending `z_order` (back-to-front); hit test in descending order.
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

/// An affine transform applied in scene coordinates before the final paint transform.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}
impl Default for Affine {
    fn default() -> Self {
        Self::identity()
    }
}
impl Affine {
    pub const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }
    pub fn compose(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }
    pub fn apply_point(self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
    pub fn invert(self) -> Option<Self> {
        let determinant = self.a * self.d - self.b * self.c;
        (determinant.is_finite() && determinant != 0.0).then(|| Self {
            a: self.d / determinant,
            b: -self.b / determinant,
            c: -self.c / determinant,
            d: self.a / determinant,
            e: (self.c * self.f - self.d * self.e) / determinant,
            f: (self.b * self.e - self.a * self.f) / determinant,
        })
    }
    pub fn is_finite(self) -> bool {
        [self.a, self.b, self.c, self.d, self.e, self.f]
            .into_iter()
            .all(f32::is_finite)
    }
    pub fn is_identity(&self) -> bool {
        *self == Self::identity()
    }
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
        #[serde(default, skip_serializing_if = "Affine::is_identity")]
        transform: Affine,
    },
    Image {
        id: String,
        z_order: u32,
        asset_id: String,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        /// Maps this local image rectangle into scene coordinates before the final paint transform.
        #[serde(default, skip_serializing_if = "Affine::is_identity")]
        transform: Affine,
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
        #[serde(default, skip_serializing_if = "Affine::is_identity")]
        transform: Affine,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    #[serde(skip)]
    pub(crate) tab: Option<super::TabStop>,
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
    pub y: f32,
}
fn is_false(value: &bool) -> bool {
    !*value
}
