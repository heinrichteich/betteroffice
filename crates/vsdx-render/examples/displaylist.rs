//! Prints a page's display list as JSON, rendered straight from the parsed
//! package. Useful for inspecting the renderer without a browser.
use vsdx_parse::parse_vsdx;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: displaylist <file.vsdx> [page]");
    let index: usize = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "0".into())
        .parse()
        .expect("page");
    let package = parse_vsdx(&std::fs::read(&path).expect("read")).expect("parse");
    let part = package
        .page_part_paths
        .get(index)
        .expect("page index")
        .clone();
    let list = vsdx_render::Renderer::new(Default::default())
        .layout_page(&package, &part)
        .expect("layout");
    println!("{}", serde_json::to_string(&list).expect("json"));
}
