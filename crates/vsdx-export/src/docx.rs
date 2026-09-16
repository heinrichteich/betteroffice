use std::collections::BTreeMap;

use vsdx_parse::VsdxPackage;
use vsdx_render::{Primitive, VsdxDisplayList};

use crate::ExportError;
use crate::Page;
use crate::geom::{
    cust_geom, emu, escape, fill_xml, has_text, line_xml, place_rect, sniff_image, wml_runs,
};
use crate::metadata::shape_data;

const CONTENT_WIDTH_IN: f64 = 6.5;

struct Media {
    part: String,
    content_type: String,
    bytes: Vec<u8>,
}

pub fn build(pages: &[Page], package: &VsdxPackage) -> Result<Vec<u8>, ExportError> {
    let data = shape_data(package)?;
    let mut media: Vec<Media> = Vec::new();
    let mut index_by_asset: BTreeMap<String, usize> = BTreeMap::new();
    for page in pages {
        collect_media(
            &page.list.primitives,
            package,
            &mut media,
            &mut index_by_asset,
        )?;
    }
    let mut parts: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".to_owned(), content_types(&media)),
        ("_rels/.rels".to_owned(), package_rels()),
        (
            "word/_rels/document.xml.rels".to_owned(),
            document_rels(&media),
        ),
        (
            "word/document.xml".to_owned(),
            document_xml(pages, &data, &media, &index_by_asset)?,
        ),
        ("word/styles.xml".to_owned(), styles_xml()),
        ("docProps/core.xml".to_owned(), core_xml()),
        ("docProps/app.xml".to_owned(), app_xml()),
    ];
    for item in &media {
        parts.push((item.part.clone(), item.bytes.clone()));
    }
    ooxml_opc::rezip_parts(&parts).map_err(ExportError::Package)
}

fn collect_media(
    primitives: &[Primitive],
    package: &VsdxPackage,
    media: &mut Vec<Media>,
    index_by_asset: &mut BTreeMap<String, usize>,
) -> Result<(), ExportError> {
    for primitive in primitives {
        match primitive {
            Primitive::Group { primitives, .. } => {
                collect_media(primitives, package, media, index_by_asset)?;
            }
            Primitive::Image { asset_id, .. } => {
                if index_by_asset.contains_key(asset_id) {
                    continue;
                }
                let Some(bytes) = package.part_bytes(asset_id) else {
                    continue;
                };
                let (ext, content_type) = sniff_image(asset_id, bytes);
                let part = format!("word/media/image{}.{ext}", media.len() + 1);
                index_by_asset.insert(asset_id.clone(), media.len());
                media.push(Media {
                    part,
                    content_type: content_type.to_owned(),
                    bytes: bytes.to_vec(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

fn scale_list(list: &VsdxDisplayList, scale: f64) -> VsdxDisplayList {
    let mut list = list.clone();
    list.width *= scale as f32;
    list.height *= scale as f32;
    scale_primitives(&mut list.primitives, scale);
    list
}

fn scale_primitives(primitives: &mut [Primitive], scale: f64) {
    for primitive in primitives {
        match primitive {
            Primitive::Shape { path, stroke, .. } => {
                for command in path.iter_mut() {
                    scale_command(command, scale);
                }
                scale_stroke(stroke, scale);
            }
            Primitive::Image {
                x,
                y,
                width,
                height,
                transform,
                ..
            } => {
                *x *= scale as f32;
                *y *= scale as f32;
                *width *= scale as f32;
                *height *= scale as f32;
                scale_affine(transform, scale);
            }
            Primitive::TextBox {
                x,
                y,
                width,
                height,
                paragraphs,
                lines,
                transform,
                ..
            } => {
                *x *= scale as f32;
                *y *= scale as f32;
                *width *= scale as f32;
                *height *= scale as f32;
                for paragraph in paragraphs {
                    for run in &mut paragraph.runs {
                        run.size_in *= scale as f32;
                        run.letter_spacing *= scale as f32;
                    }
                }
                for line in lines {
                    line.x *= scale as f32;
                    line.y *= scale as f32;
                    line.width *= scale as f32;
                    line.height *= scale as f32;
                    for stop in &mut line.caret_stops {
                        stop.x *= scale as f32;
                        stop.y *= scale as f32;
                    }
                }
                scale_affine(transform, scale);
            }
            Primitive::Placeholder {
                x,
                y,
                width,
                height,
                ..
            } => {
                *x *= scale as f32;
                *y *= scale as f32;
                *width *= scale as f32;
                *height *= scale as f32;
            }
            Primitive::Group {
                primitives,
                transform,
                ..
            } => {
                scale_primitives(primitives, scale);
                scale_affine(transform, scale);
            }
        }
    }
}

fn scale_command(command: &mut ooxml_drawingml::GeometryPathCommand, scale: f64) {
    use ooxml_drawingml::GeometryPathCommand as Command;
    let factor = scale;
    match command {
        Command::Move { x, y } | Command::Line { x, y } => {
            *x *= factor;
            *y *= factor;
        }
        Command::Quad { cpx, cpy, x, y } => {
            *cpx *= factor;
            *cpy *= factor;
            *x *= factor;
            *y *= factor;
        }
        Command::Cubic {
            cp1x,
            cp1y,
            cp2x,
            cp2y,
            x,
            y,
        } => {
            *cp1x *= factor;
            *cp1y *= factor;
            *cp2x *= factor;
            *cp2y *= factor;
            *x *= factor;
            *y *= factor;
        }
        Command::Close => {}
    }
}

fn scale_stroke(stroke: &mut Option<vsdx_render::Stroke>, scale: f64) {
    if let Some(stroke) = stroke {
        stroke.width *= scale as f32;
    }
}

fn scale_affine(transform: &mut vsdx_render::Affine, scale: f64) {
    transform.e *= scale as f32;
    transform.f *= scale as f32;
}

fn fit_scale(page: &Page) -> f64 {
    (CONTENT_WIDTH_IN / page.width_in).min(1.0)
}

fn document_xml(
    pages: &[Page],
    data: &[crate::ShapeDatum],
    media: &[Media],
    index_by_asset: &BTreeMap<String, usize>,
) -> Result<Vec<u8>, ExportError> {
    let mut body = String::new();
    let mut doc_id: u32 = 1;
    for page in pages {
        let name = escape(&page.name);
        body.push_str(&format!(
            "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr><w:r><w:t xml:space=\"preserve\">Page {name}</w:t></w:r></w:p>"
        ));
        let scale = fit_scale(page);
        let list = scale_list(&page.list, scale);
        let page_height = f64::from(list.height) / 96.0;
        let width = emu(f64::from(list.width) / 96.0).max(1);
        let height = emu(page_height).max(1);
        let mut members = String::new();
        let mut ordered = Vec::new();
        flat(&list.primitives, &mut ordered);
        for primitive in ordered {
            members.push_str(&member_xml(primitive, page_height)?);
        }
        body.push_str(&format!(
            "<w:p><w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{width}\" cy=\"{height}\"/><wp:docPr id=\"{doc_id}\" name=\"{name}\"/><a:graphic><a:graphicData uri=\"http://schemas.microsoft.com/office/word/2010/wordprocessingGroup\"><wpg:wgp><wpg:cNvGrpSpPr/><wpg:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{width}\" cy=\"{height}\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"{width}\" cy=\"{height}\"/></a:xfrm></wpg:grpSpPr>{members}</wpg:wgp></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"
        ));
        doc_id += 1;
        for primitive in flat_images(&list.primitives) {
            body.push_str(&image_paragraph(
                primitive,
                &mut doc_id,
                media,
                index_by_asset,
            )?);
        }
        let rows: Vec<&crate::ShapeDatum> = data
            .iter()
            .filter(|datum| datum.page == page.name)
            .collect();
        if !rows.is_empty() {
            let title = escape(&page.name);
            body.push_str(&format!(
                "<w:p><w:pPr><w:pStyle w:val=\"Heading2\"/></w:pPr><w:r><w:t xml:space=\"preserve\">Shape data for {title}</w:t></w:r></w:p>"
            ));
            body.push_str(&table_xml(&rows));
        }
    }
    Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\" xmlns:wpg=\"http://schemas.microsoft.com/office/word/2010/wordprocessingGroup\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>").into_bytes())
}

fn flat<'a>(primitives: &'a [Primitive], out: &mut Vec<&'a Primitive>) {
    let mut ordered: Vec<&Primitive> = primitives.iter().collect();
    ordered.sort_by_key(|primitive| match primitive {
        Primitive::Shape { z_order, .. }
        | Primitive::Image { z_order, .. }
        | Primitive::TextBox { z_order, .. }
        | Primitive::Placeholder { z_order, .. }
        | Primitive::Group { z_order, .. } => *z_order,
    });
    for primitive in ordered {
        match primitive {
            Primitive::Group { primitives, .. } => flat(primitives, out),
            Primitive::Image { .. } => {}
            _ => out.push(primitive),
        }
    }
}

fn flat_images(primitives: &[Primitive]) -> Vec<&Primitive> {
    let mut out = Vec::new();
    let mut visit: Vec<&[Primitive]> = vec![primitives];
    while let Some(list) = visit.pop() {
        for primitive in list {
            match primitive {
                Primitive::Group { primitives, .. } => visit.push(primitives),
                Primitive::Image { .. } => out.push(primitive),
                _ => {}
            }
        }
    }
    out
}

fn member_xml(primitive: &Primitive, page_height: f64) -> Result<String, ExportError> {
    match primitive {
        Primitive::Shape {
            path, fill, stroke, ..
        } => {
            let Some(geom) = cust_geom(path, page_height) else {
                return Ok(String::new());
            };
            let fill = fill_xml(fill);
            let line = line_xml(stroke).unwrap_or_default();
            Ok(format!(
                "<wps:wsp><wps:cNvSpPr/><wps:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:pathLst><a:path w=\"{}\" h=\"{}\">{}</a:path></a:pathLst></a:custGeom>{fill}{line}</wps:spPr><wps:txbx><w:txbxContent><w:p/></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp>",
                geom.x, geom.y, geom.w, geom.h, geom.w, geom.h, geom.body
            ))
        }
        Primitive::TextBox {
            x,
            y,
            width,
            height,
            paragraphs,
            transform,
            ..
        } => {
            let Some(placed) = place_rect(*x, *y, *width, *height, *transform, page_height) else {
                return Ok(String::new());
            };
            let mut text = wml_runs(paragraphs);
            if !has_text(paragraphs) {
                text = "<w:p/>".to_owned();
            }
            Ok(format!(
                "<wps:wsp><wps:cNvSpPr txBox=\"1\"/><wps:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></wps:spPr><wps:txbx><w:txbxContent>{text}</w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp>",
                wps_xfrm(&placed)
            ))
        }
        Primitive::Placeholder {
            x,
            y,
            width,
            height,
            reason,
            ..
        } => {
            let label = escape(reason);
            Ok(format!(
                "<wps:wsp><wps:cNvSpPr txBox=\"1\"/><wps:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"8A94A6\"/></a:solidFill><a:prstDash val=\"dash\"/></a:ln></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:cs=\"Calibri\"/><w:color w:val=\"5D6675\"/><w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr><w:t xml:space=\"preserve\">{label}</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp>",
                emu(f64::from(*x)),
                emu(page_height - f64::from(*y) - f64::from(*height)),
                emu(f64::from(*width)).max(1),
                emu(f64::from(*height)).max(1),
            ))
        }
        Primitive::Image { .. } | Primitive::Group { .. } => Ok(String::new()),
    }
}

fn take_id(next_id: &mut u32) -> u32 {
    let id = (*next_id).max(1);
    *next_id = next_id.saturating_add(1).max(1);
    id
}

fn wps_xfrm(placed: &crate::geom::Placed) -> String {
    if placed.rot == 0 {
        format!(
            "<a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
            placed.x, placed.y, placed.w, placed.h
        )
    } else {
        format!(
            "<a:xfrm rot=\"{}\"><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm>",
            placed.rot, placed.x, placed.y, placed.w, placed.h
        )
    }
}

fn image_paragraph(
    primitive: &Primitive,
    doc_id: &mut u32,
    media: &[Media],
    index_by_asset: &BTreeMap<String, usize>,
) -> Result<String, ExportError> {
    let Primitive::Image {
        id,
        width,
        height,
        asset_id,
        ..
    } = primitive
    else {
        return Ok(String::new());
    };
    let Some(media_index) = index_by_asset.get(asset_id) else {
        return Ok(String::new());
    };
    let _ = &media[*media_index];
    let embed = format!("rId{}", media_index + 2);
    let name = escape(id);
    let scale = (CONTENT_WIDTH_IN / f64::from(*width).max(0.01)).min(1.0);
    let w = emu(f64::from(*width) * scale).max(1);
    let h = emu(f64::from(*height) * scale).max(1);
    let id_value = take_id(doc_id);
    Ok(format!(
        "<w:p><w:r><w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{w}\" cy=\"{h}\"/><wp:docPr id=\"{id_value}\" name=\"{name}\"/><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id_value}\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{embed}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"
    ))
}

fn table_xml(rows: &[&crate::ShapeDatum]) -> String {
    let mut body = String::from(
        "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:color=\"8A94A6\"/><w:left w:val=\"single\" w:sz=\"4\" w:color=\"8A94A6\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:color=\"8A94A6\"/><w:right w:val=\"single\" w:sz=\"4\" w:color=\"8A94A6\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:color=\"8A94A6\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:color=\"8A94A6\"/></w:tblBorders></w:tblPr><w:tblGrid><w:gridCol w:w=\"2000\"/><w:gridCol w:w=\"3000\"/><w:gridCol w:w=\"4360\"/></w:tblGrid>",
    );
    body.push_str(&table_row(&["Shape", "Label", "Value"], true));
    for datum in rows {
        body.push_str(&table_row(
            &[&datum.shape, &datum.label, &datum.value],
            false,
        ));
    }
    body.push_str("</w:tbl>");
    body
}

fn table_row(cells: &[&str], header: bool) -> String {
    let mut out = String::from("<w:tr>");
    for cell in cells {
        let text = escape(cell);
        let run = if header {
            format!(
                "<w:r><w:rPr><w:b/><w:bCs/></w:rPr><w:t xml:space=\"preserve\">{text}</w:t></w:r>"
            )
        } else {
            format!("<w:r><w:t xml:space=\"preserve\">{text}</w:t></w:r>")
        };
        out.push_str(&format!("<w:tc><w:p>{run}</w:p></w:tc>"));
    }
    out.push_str("</w:tr>");
    out
}

fn content_types(media: &[Media]) -> Vec<u8> {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>",
    );
    let mut extensions: Vec<String> = Vec::new();
    for item in media {
        let ext = item.part.rsplit('.').next().unwrap_or("bin").to_owned();
        if !extensions.contains(&ext) {
            extensions.push(ext);
        }
    }
    for ext in &extensions {
        let content_type = media
            .iter()
            .find(|item| item.part.ends_with(&format!(".{ext}")))
            .map(|item| item.content_type.clone())
            .unwrap_or_else(|| "application/octet-stream".to_owned());
        out.push_str(&format!(
            "<Default Extension=\"{ext}\" ContentType=\"{content_type}\"/>"
        ));
    }
    out.push_str("<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>");
    for item in media {
        out.push_str(&format!(
            "<Override PartName=\"/{}\" ContentType=\"{}\"/>",
            item.part, item.content_type
        ));
    }
    out.push_str("</Types>");
    out.into_bytes()
}

fn package_rels() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>".to_owned().into_bytes()
}

fn document_rels(media: &[Media]) -> Vec<u8> {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>",
    );
    for (index, item) in media.iter().enumerate() {
        let id = index + 2;
        let target = item.part.strip_prefix("word/").unwrap_or(&item.part);
        out.push_str(&format!("<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"{target}\"/>"));
    }
    out.push_str("</Relationships>");
    out.into_bytes()
}

fn styles_xml() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:cs=\"Calibri\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"259\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:cs=\"Calibri\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:rPr><w:b/><w:bCs/><w:color w:val=\"1F2937\"/><w:sz w:val=\"32\"/><w:szCs w:val=\"32\"/></w:rPr></w:style><w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:rPr><w:b/><w:bCs/><w:color w:val=\"374151\"/><w:sz w:val=\"26\"/><w:szCs w:val=\"26\"/></w:rPr></w:style></w:styles>".to_owned().into_bytes()
}

fn core_xml() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Diagram export</dc:title><dc:creator>BetterOffice</dc:creator><cp:revision>1</cp:revision></cp:coreProperties>".to_owned().into_bytes()
}

fn app_xml() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>BetterOffice</Application></Properties>".to_owned().into_bytes()
}
