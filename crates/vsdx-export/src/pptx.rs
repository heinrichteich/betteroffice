use std::collections::BTreeMap;

use vsdx_parse::VsdxPackage;
use vsdx_render::Primitive;

use crate::ExportError;
use crate::Page;
use crate::geom::{
    Placed, cust_geom, dml_paragraphs, emu, escape, fill_xml, line_xml, place_rect, sniff_image,
};

struct Media {
    part: String,
    content_type: String,
    bytes: Vec<u8>,
}

pub fn build(pages: &[Page], package: &VsdxPackage) -> Result<Vec<u8>, ExportError> {
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
    let max_w = pages
        .iter()
        .map(|page| emu(page.width_in))
        .max()
        .unwrap_or(11_433_600);
    let max_h = pages
        .iter()
        .map(|page| emu(page.height_in))
        .max()
        .unwrap_or(8_575_200);
    let mut parts: Vec<(String, Vec<u8>)> = vec![
        (
            "[Content_Types].xml".to_owned(),
            content_types(pages, &media),
        ),
        ("_rels/.rels".to_owned(), package_rels()),
        (
            "ppt/presentation.xml".to_owned(),
            presentation_xml(pages, max_w, max_h),
        ),
        (
            "ppt/_rels/presentation.xml.rels".to_owned(),
            presentation_rels(pages),
        ),
        ("ppt/slideMasters/slideMaster1.xml".to_owned(), master_xml()),
        (
            "ppt/slideMasters/_rels/slideMaster1.xml.rels".to_owned(),
            master_rels(),
        ),
        ("ppt/slideLayouts/slideLayout1.xml".to_owned(), layout_xml()),
        (
            "ppt/slideLayouts/_rels/slideLayout1.xml.rels".to_owned(),
            layout_rels(),
        ),
        ("ppt/theme/theme1.xml".to_owned(), theme_xml()),
        ("docProps/core.xml".to_owned(), core_xml()),
        ("docProps/app.xml".to_owned(), app_xml(pages)),
    ];
    for (slide_index, page) in pages.iter().enumerate() {
        let number = slide_index + 1;
        let used = used_media(&page.list.primitives, &index_by_asset);
        parts.push((
            format!("ppt/slides/slide{number}.xml"),
            slide_xml(page, &index_by_asset)?,
        ));
        parts.push((
            format!("ppt/slides/_rels/slide{number}.xml.rels"),
            slide_rels(&media, &used),
        ));
    }
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
                let part = format!("ppt/media/image{}.{ext}", media.len() + 1);
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

fn flat<'a>(primitives: &'a [Primitive], out: &mut Vec<&'a Primitive>) {
    let mut ordered: Vec<&Primitive> = primitives.iter().collect();
    ordered.sort_by_key(|primitive| z_order(primitive));
    for primitive in ordered {
        match primitive {
            Primitive::Group { primitives, .. } => flat(primitives, out),
            _ => out.push(primitive),
        }
    }
}

fn z_order(primitive: &Primitive) -> u32 {
    match primitive {
        Primitive::Shape { z_order, .. }
        | Primitive::Image { z_order, .. }
        | Primitive::TextBox { z_order, .. }
        | Primitive::Placeholder { z_order, .. }
        | Primitive::Group { z_order, .. } => *z_order,
    }
}

fn used_media(primitives: &[Primitive], index_by_asset: &BTreeMap<String, usize>) -> Vec<usize> {
    let mut used = Vec::new();
    let mut visit: Vec<&[Primitive]> = vec![primitives];
    while let Some(list) = visit.pop() {
        for primitive in list {
            match primitive {
                Primitive::Group { primitives, .. } => visit.push(primitives),
                Primitive::Image { asset_id, .. } => {
                    if let Some(index) = index_by_asset.get(asset_id)
                        && !used.contains(index)
                    {
                        used.push(*index);
                    }
                }
                _ => {}
            }
        }
    }
    used.sort_unstable();
    used
}

fn slide_xml(
    page: &Page,
    index_by_asset: &BTreeMap<String, usize>,
) -> Result<Vec<u8>, ExportError> {
    let page_height = page.height_in;
    let mut shapes = String::new();
    let mut next_id: u32 = 2;
    let mut ordered = Vec::new();
    flat(&page.list.primitives, &mut ordered);
    for primitive in ordered {
        match primitive {
            Primitive::Shape {
                id,
                path,
                fill,
                stroke,
                ..
            } => {
                let Some(geom) = cust_geom(path, page_height) else {
                    continue;
                };
                let name = escape(id);
                let fill = fill_xml(fill);
                let line = line_xml(stroke).unwrap_or_default();
                let id_value = next_id;
                next_id += 1;
                shapes.push_str(&format!(
                    "<p:sp><p:nvSpPr><p:cNvPr id=\"{id_value}\" name=\"{name}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:pathLst><a:path w=\"{}\" h=\"{}\">{}</a:path></a:pathLst></a:custGeom>{fill}{line}</p:spPr></p:sp>",
                    geom.x, geom.y, geom.w, geom.h, geom.w, geom.h, geom.body
                ));
            }
            Primitive::TextBox {
                id,
                x,
                y,
                width,
                height,
                paragraphs,
                transform,
                ..
            } => {
                let Some(placed) = place_rect(*x, *y, *width, *height, *transform, page_height)
                else {
                    continue;
                };
                let name = escape(id);
                let mut body = dml_paragraphs(paragraphs);
                if body.is_empty() {
                    body.push_str("<a:p/>");
                }
                let id_value = next_id;
                next_id += 1;
                shapes.push_str(&format!(
                    "<p:sp><p:nvSpPr><p:cNvPr id=\"{id_value}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noChangeArrowheads=\"1\"/></p:cNvSpPr><p:nvPr/></p:nvSpPr><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp>",
                    xfrm(&placed)
                ));
            }
            Primitive::Placeholder {
                id,
                x,
                y,
                width,
                height,
                reason,
                ..
            } => {
                let name = escape(id);
                let label = escape(reason);
                let id_value = next_id;
                next_id += 1;
                shapes.push_str(&format!(
                    "<p:sp><p:nvSpPr><p:cNvPr id=\"{id_value}\" name=\"{name}\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"{}\" y=\"{}\"/><a:ext cx=\"{}\" cy=\"{}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/><a:ln w=\"12700\"><a:solidFill><a:srgbClr val=\"8A94A6\"/></a:solidFill><a:prstDash val=\"dash\"/></a:ln></p:spPr><p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr sz=\"1200\"><a:solidFill><a:srgbClr val=\"5D6675\"/></a:solidFill><a:latin typeface=\"Calibri\"/></a:rPr><a:t>{label}</a:t></a:r></a:p></p:txBody></p:sp>",
                    emu(f64::from(*x)),
                    emu(page_height - f64::from(*y) - f64::from(*height)),
                    emu(f64::from(*width)).max(1),
                    emu(f64::from(*height)).max(1),
                ));
            }
            Primitive::Image {
                id,
                x,
                y,
                width,
                height,
                asset_id,
                transform,
                ..
            } => {
                let Some(media_index) = index_by_asset.get(asset_id) else {
                    continue;
                };
                let Some(placed) = place_rect(*x, *y, *width, *height, *transform, page_height)
                else {
                    continue;
                };
                let name = escape(id);
                let embed = format!("rId{}", media_index + 2);
                let id_value = next_id;
                next_id += 1;
                shapes.push_str(&format!(
                    "<p:pic><p:nvPicPr><p:cNvPr id=\"{id_value}\" name=\"{name}\"/><p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"{embed}\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
                    xfrm(&placed)
                ));
            }
            Primitive::Group { .. } => {}
        }
    }
    let name = escape(&page.name);
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld name=\"{name}\"><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"
    )
    .into_bytes())
}

fn xfrm(placed: &Placed) -> String {
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

fn content_types(pages: &[Page], media: &[Media]) -> Vec<u8> {
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
    out.push_str("<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/><Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/><Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/><Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/><Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>");
    for (index, _) in pages.iter().enumerate() {
        let number = index + 1;
        out.push_str(&format!("<Override PartName=\"/ppt/slides/slide{number}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"));
    }
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
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/><Relationship Id=\"rId3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties\" Target=\"docProps/app.xml\"/></Relationships>".to_owned().into_bytes()
}

fn presentation_xml(pages: &[Page], max_w: i64, max_h: i64) -> Vec<u8> {
    let mut ids = String::new();
    for (index, _) in pages.iter().enumerate() {
        let number = index + 1;
        let id = 256 + index as u32;
        ids.push_str(&format!("<p:sldId id=\"{id}\" r:id=\"rId{number}\"/>"));
    }
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId{}\"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"{max_w}\" cy=\"{max_h}\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>", pages.len() + 1).into_bytes()
}

fn presentation_rels(pages: &[Page]) -> Vec<u8> {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (index, _) in pages.iter().enumerate() {
        let number = index + 1;
        out.push_str(&format!("<Relationship Id=\"rId{number}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{number}.xml\"/>"));
    }
    let master = pages.len() + 1;
    out.push_str(&format!("<Relationship Id=\"rId{master}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/></Relationships>"));
    out.into_bytes()
}

fn slide_rels(media: &[Media], used: &[usize]) -> Vec<u8> {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>",
    );
    for index in used {
        let item = &media[*index];
        let id = index + 2;
        let relative = relative_media(&item.part);
        out.push_str(&format!("<Relationship Id=\"rId{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"{relative}\"/>"));
    }
    out.push_str("</Relationships>");
    out.into_bytes()
}

fn relative_media(part: &str) -> String {
    part.strip_prefix("ppt/")
        .map(|rest| format!("../{rest}"))
        .unwrap_or_else(|| part.to_owned())
}

fn master_xml() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:sldMaster xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr><a:defRPr sz=\"4400\" b=\"1\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"Calibri\"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr><a:defRPr sz=\"3200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"Calibri\"/></a:defRPr></a:lvl1pPr></p:bodyStyle><p:otherStyle><a:lvl1pPr><a:defRPr sz=\"1800\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"Calibri\"/></a:defRPr></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>".to_owned().into_bytes()
}

fn master_rels() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/></Relationships>".to_owned().into_bytes()
}

fn layout_xml() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><p:sldLayout xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" type=\"blank\" preserve=\"1\"><p:cSld name=\"Blank\"><p:spTree><p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>".to_owned().into_bytes()
}

fn layout_rels() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/></Relationships>".to_owned().into_bytes()
}

fn theme_xml() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Export\"><a:themeElements><a:clrScheme name=\"Export\"><a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1><a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"1F2937\"/></a:dk2><a:lt2><a:srgbClr val=\"E5E7EB\"/></a:lt2><a:accent1><a:srgbClr val=\"2563EB\"/></a:accent1><a:accent2><a:srgbClr val=\"059669\"/></a:accent2><a:accent3><a:srgbClr val=\"D97706\"/></a:accent3><a:accent4><a:srgbClr val=\"DC2626\"/></a:accent4><a:accent5><a:srgbClr val=\"7C3AED\"/></a:accent5><a:accent6><a:srgbClr val=\"0891B2\"/></a:accent6><a:hlink><a:srgbClr val=\"2563EB\"/></a:hlink><a:folHlink><a:srgbClr val=\"7C3AED\"/></a:folHlink></a:clrScheme><a:fontScheme name=\"Export\"><a:majorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme><a:fmtScheme name=\"Export\"><a:fillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"/></a:gs><a:gs pos=\"50000\"><a:schemeClr val=\"phClr\"/></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"/></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill><a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"/></a:gs><a:gs pos=\"50000\"><a:schemeClr val=\"phClr\"/></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"/></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill></a:fillStyleLst><a:lnStyleLst><a:ln w=\"12700\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln><a:ln w=\"19050\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln><a:ln w=\"25400\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst><a:outerShdw blurRad=\"57150\" dist=\"19050\" dir=\"5400000\" algn=\"ctr\" rotWithShape=\"0\"><a:srgbClr val=\"000000\"><a:alpha val=\"63000\"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:gradFill rotWithShape=\"1\"><a:gsLst><a:gs pos=\"0\"><a:schemeClr val=\"phClr\"/></a:gs><a:gs pos=\"50000\"><a:schemeClr val=\"phClr\"/></a:gs><a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"/></a:gs></a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>".to_owned().into_bytes()
}

fn core_xml() -> Vec<u8> {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Diagram export</dc:title><dc:creator>BetterOffice</dc:creator><cp:revision>1</cp:revision></cp:coreProperties>".to_owned().into_bytes()
}

fn app_xml(pages: &[Page]) -> Vec<u8> {
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>BetterOffice</Application><PresentationFormat>On-screen Show (4:3)</PresentationFormat><Slides>{}</Slides></Properties>", pages.len()).into_bytes()
}
