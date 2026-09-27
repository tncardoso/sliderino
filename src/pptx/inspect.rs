//! Reads a PPTX back: its parts, their relationships, a check of the
//! package, and a summary of the slides in slide units. The debug tools and
//! the parity tests use it to compare a deck with the presentation it
//! comes from.

use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read as _};

use roxmltree::{Document, Node};

use super::package::{REL, rels_name};
use super::{A_NS, P_NS, R_NS};

#[derive(Debug)]
pub enum InspectError {
    Zip(String),
    MissingPart(String),
    Xml { part: String, message: String },
}

impl std::fmt::Display for InspectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InspectError::Zip(message) => write!(f, "not a zip archive: {message}"),
            InspectError::MissingPart(part) => write!(f, "the part {part} is missing"),
            InspectError::Xml { part, message } => write!(f, "{part}: {message}"),
        }
    }
}

impl std::error::Error for InspectError {}

/// One relationship of a part, with its target resolved to a part name.
#[derive(Clone, Debug, PartialEq)]
pub struct Relationship {
    pub id: String,
    /// The type, without the common prefix when it has it.
    pub kind: String,
    /// The part name, or the URL of an external target.
    pub target: String,
    pub external: bool,
}

/// The parts of a PPTX file, in archive order.
pub struct Deck {
    names: Vec<String>,
    parts: BTreeMap<String, Vec<u8>>,
}

impl Deck {
    pub fn read(bytes: &[u8]) -> Result<Deck, InspectError> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|error| InspectError::Zip(error.to_string()))?;
        let mut names = Vec::new();
        let mut parts = BTreeMap::new();
        for index in 0..zip.len() {
            let mut entry = zip
                .by_index(index)
                .map_err(|error| InspectError::Zip(error.to_string()))?;
            if entry.is_dir() {
                continue;
            }
            let mut data = Vec::new();
            entry
                .read_to_end(&mut data)
                .map_err(|error| InspectError::Zip(error.to_string()))?;
            names.push(entry.name().to_string());
            parts.insert(entry.name().to_string(), data);
        }
        Ok(Deck { names, parts })
    }

    pub fn load(path: &std::path::Path) -> Result<Deck, Box<dyn std::error::Error>> {
        Ok(Deck::read(&std::fs::read(path)?)?)
    }

    /// Part names in archive order.
    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn part(&self, name: &str) -> Option<&[u8]> {
        self.parts.get(name).map(Vec::as_slice)
    }

    pub fn text(&self, name: &str) -> Result<&str, InspectError> {
        let bytes = self
            .part(name)
            .ok_or_else(|| InspectError::MissingPart(name.to_string()))?;
        std::str::from_utf8(bytes).map_err(|error| InspectError::Xml {
            part: name.to_string(),
            message: error.to_string(),
        })
    }

    /// Parses an XML part and gives it to `read`.
    pub fn with_xml<T>(
        &self,
        name: &str,
        read: impl FnOnce(&Document) -> T,
    ) -> Result<T, InspectError> {
        let text = self.text(name)?;
        let document = Document::parse(text).map_err(|error| InspectError::Xml {
            part: name.to_string(),
            message: error.to_string(),
        })?;
        Ok(read(&document))
    }

    /// The relationships of `part` (`""` for the package). A part without
    /// a `.rels` part has none.
    pub fn rels(&self, part: &str) -> Result<Vec<Relationship>, InspectError> {
        let name = rels_name(part);
        if self.part(&name).is_none() {
            return Ok(Vec::new());
        }
        let folder = part.rsplit_once('/').map_or("", |(folder, _)| folder);
        self.with_xml(&name, |document| {
            document
                .root_element()
                .children()
                .filter(|node| node.has_tag_name("Relationship"))
                .map(|node| {
                    let kind = node.attribute("Type").unwrap_or_default();
                    let target = node.attribute("Target").unwrap_or_default();
                    let external = node.attribute("TargetMode") == Some("External");
                    Relationship {
                        id: node.attribute("Id").unwrap_or_default().to_string(),
                        kind: kind
                            .strip_prefix(REL)
                            .map(|kind| kind.trim_start_matches('/'))
                            .unwrap_or(kind)
                            .to_string(),
                        target: if external {
                            target.to_string()
                        } else {
                            resolve(folder, target)
                        },
                        external,
                    }
                })
                .collect()
        })
    }

    /// The target part of relationship `id` of `part`.
    pub fn target(&self, part: &str, id: &str) -> Result<Option<String>, InspectError> {
        Ok(self
            .rels(part)?
            .into_iter()
            .find(|rel| rel.id == id)
            .map(|rel| rel.target))
    }

    /// The main part: the target of the package's officeDocument
    /// relationship.
    pub fn presentation_part(&self) -> Result<String, InspectError> {
        self.rels("")?
            .into_iter()
            .find(|rel| rel.kind == "officeDocument")
            .map(|rel| rel.target)
            .ok_or_else(|| InspectError::MissingPart("the officeDocument relationship".into()))
    }

    /// The slide part names in presentation order.
    pub fn slides(&self) -> Result<Vec<String>, InspectError> {
        let main = self.presentation_part()?;
        let ids: Vec<String> = self.with_xml(&main, |document| {
            document
                .descendants()
                .filter(|node| node.tag_name().name() == "sldId")
                .filter_map(|node| node.attribute((R_NS, "id")).map(str::to_string))
                .collect()
        })?;
        let rels = self.rels(&main)?;
        ids.iter()
            .map(|id| {
                rels.iter()
                    .find(|rel| &rel.id == id)
                    .map(|rel| rel.target.clone())
                    .ok_or_else(|| InspectError::MissingPart(format!("{main} relationship {id}")))
            })
            .collect()
    }

    /// The content type of `part`: its Override, or the Default of its
    /// extension.
    pub fn content_type(&self, part: &str) -> Result<Option<String>, InspectError> {
        self.with_xml("[Content_Types].xml", |document| {
            let root = document.root_element();
            let wanted = format!("/{part}");
            let over = root
                .children()
                .filter(|node| node.has_tag_name("Override"))
                .find(|node| {
                    node.attribute("PartName")
                        .is_some_and(|name| name.eq_ignore_ascii_case(&wanted))
                });
            if let Some(node) = over {
                return node.attribute("ContentType").map(str::to_string);
            }
            let extension = part.rsplit('.').next().unwrap_or_default();
            root.children()
                .filter(|node| node.has_tag_name("Default"))
                .find(|node| {
                    node.attribute("Extension")
                        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
                })
                .and_then(|node| node.attribute("ContentType").map(str::to_string))
        })
    }

    /// The problems of the package. An empty list means the package is
    /// consistent (this does not check the XML against the schema).
    pub fn check(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.names.first().map(String::as_str) != Some("[Content_Types].xml") {
            problems.push("[Content_Types].xml is not the first entry".to_string());
        }
        if self.part("[Content_Types].xml").is_none() {
            problems.push("[Content_Types].xml is missing".to_string());
            return problems;
        }
        let overrides: Vec<String> = match self.with_xml("[Content_Types].xml", |document| {
            document
                .root_element()
                .children()
                .filter(|node| node.has_tag_name("Override"))
                .filter_map(|node| node.attribute("PartName"))
                .map(|name| name.trim_start_matches('/').to_string())
                .collect()
        }) {
            Ok(overrides) => overrides,
            Err(error) => {
                problems.push(error.to_string());
                return problems;
            }
        };
        for name in overrides {
            if self.part(&name).is_none() {
                problems.push(format!("the Override for {name} names no part"));
            }
        }

        for name in &self.names {
            if name == "[Content_Types].xml" {
                continue;
            }
            match self.content_type(name) {
                Ok(Some(_)) => {}
                Ok(None) => problems.push(format!("{name} has no content type")),
                Err(error) => problems.push(error.to_string()),
            }
            let xml = name.ends_with(".xml") || name.ends_with(".rels");
            if xml && let Err(error) = self.with_xml(name, |_| ()) {
                problems.push(error.to_string());
            }
        }

        for name in &self.names {
            if name.ends_with(".rels") || name == "[Content_Types].xml" {
                continue;
            }
            let rels = match self.rels(name) {
                Ok(rels) => rels,
                Err(error) => {
                    problems.push(error.to_string());
                    continue;
                }
            };
            let mut ids = HashSet::new();
            for rel in &rels {
                if !ids.insert(rel.id.clone()) {
                    problems.push(format!("{name}: relationship id {} repeats", rel.id));
                }
                if !rel.external && self.part(&rel.target).is_none() {
                    problems.push(format!(
                        "{name}: relationship {} points to the missing part {}",
                        rel.id, rel.target
                    ));
                }
            }
            if !name.ends_with(".xml") {
                continue;
            }
            let used = self.with_xml(name, |document| {
                let mut used = Vec::new();
                for node in document.descendants().filter(Node::is_element) {
                    for attribute in node.attributes() {
                        if attribute.namespace() == Some(R_NS) {
                            used.push(attribute.value().to_string());
                        }
                    }
                }
                used
            });
            for id in used.unwrap_or_default() {
                if !ids.contains(&id) {
                    problems.push(format!("{name}: r:{id} has no relationship"));
                }
            }
        }

        match self.slides() {
            Ok(slides) => {
                for slide in slides {
                    problems.extend(self.check_slide(&slide));
                }
            }
            Err(error) => problems.push(error.to_string()),
        }
        problems
    }

    /// Shape ids must be unique on a slide.
    fn check_slide(&self, slide: &str) -> Vec<String> {
        self.with_xml(slide, |document| {
            let mut problems = Vec::new();
            let mut ids = HashSet::new();
            for node in document
                .descendants()
                .filter(|node| node.has_tag_name((P_NS, "cNvPr")))
            {
                let id = node.attribute("id").unwrap_or_default();
                if !ids.insert(id.to_string()) {
                    problems.push(format!("{slide}: shape id {id} repeats"));
                }
            }
            problems
        })
        .unwrap_or_else(|error| vec![error.to_string()])
    }
}

/// Resolves a relative `target` from the folder of its source part.
fn resolve(folder: &str, target: &str) -> String {
    if let Some(absolute) = target.strip_prefix('/') {
        return absolute.to_string();
    }
    let mut segments: Vec<&str> = folder.split('/').filter(|part| !part.is_empty()).collect();
    for segment in target.split('/') {
        match segment {
            ".." => {
                segments.pop();
            }
            "." | "" => {}
            _ => segments.push(segment),
        }
    }
    segments.join("/")
}

/// The XML of a part, indented, for reading.
pub fn pretty(xml: &str) -> Result<String, roxmltree::Error> {
    let document = Document::parse(xml)?;
    let mut out = String::new();
    write_node(&mut out, document.root_element(), 0);
    Ok(out)
}

fn write_node(out: &mut String, node: Node, depth: usize) {
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push('<');
    out.push_str(&qualified(node));
    for attribute in node.attributes() {
        let prefix = attribute
            .namespace()
            .and_then(|namespace| node.lookup_prefix(namespace))
            .map(|prefix| format!("{prefix}:"))
            .unwrap_or_default();
        out.push_str(&format!(
            " {prefix}{}=\"{}\"",
            attribute.name(),
            attribute.value()
        ));
    }
    let children: Vec<Node> = node
        .children()
        .filter(|child| child.is_element() || child.text().is_some_and(|t| !t.trim().is_empty()))
        .collect();
    if children.is_empty() {
        out.push_str("/>\n");
        return;
    }
    if let [only] = children.as_slice()
        && only.is_text()
    {
        out.push_str(&format!(
            ">{}</{}>\n",
            only.text().unwrap_or_default(),
            qualified(node)
        ));
        return;
    }
    out.push_str(">\n");
    for child in children {
        if child.is_element() {
            write_node(out, child, depth + 1);
        } else {
            out.push_str(&format!("{indent}  {}\n", child.text().unwrap_or_default()));
        }
    }
    out.push_str(&format!("{indent}</{}>\n", qualified(node)));
}

fn qualified(node: Node) -> String {
    let name = node.tag_name();
    match name
        .namespace()
        .and_then(|namespace| node.lookup_prefix(namespace))
    {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}:{}", name.name()),
        _ => name.name().to_string(),
    }
}

/// Elements of `node` named `name` in the DrawingML namespace.
pub fn a<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.has_tag_name((A_NS, name)))
}

/// The first child of `node` named `name` in the PresentationML namespace.
pub fn p<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children()
        .find(|child| child.has_tag_name((P_NS, name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_targets() {
        assert_eq!(
            resolve("ppt/slides", "../slideLayouts/slideLayout1.xml"),
            "ppt/slideLayouts/slideLayout1.xml"
        );
        assert_eq!(resolve("", "ppt/presentation.xml"), "ppt/presentation.xml");
        assert_eq!(resolve("ppt", "/docProps/app.xml"), "docProps/app.xml");
    }

    #[test]
    fn pretty_prints_nested_parts() {
        let text = pretty(r#"<a xmlns:x="u"><x:b k="1">t</x:b><c/></a>"#).unwrap();
        assert_eq!(text, "<a>\n  <x:b k=\"1\">t</x:b>\n  <c/>\n</a>\n");
    }
}

/// What a shape of a slide is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeKind {
    Shape,
    Connector,
    Group,
    Picture,
    Frame,
}

/// A color with its alpha (0-1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub rgb: u32,
    pub alpha: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FillSummary {
    None,
    Solid(Color),
    Linear {
        /// Degrees.
        angle: f32,
        scaled: bool,
        stops: Vec<(f32, Color)>,
    },
    Path {
        path: String,
        /// Insets l, t, r, b as fractions.
        fill_to: [f32; 4],
        tile: [f32; 4],
        stops: Vec<(f32, Color)>,
    },
    /// A picture: the part of the image and its placement.
    Blip {
        part: String,
        alpha: f32,
        /// Insets l, t, r, b as fractions: srcRect, and fillRect.
        source: [f32; 4],
        fill: [f32; 4],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct LineSummary {
    /// Slide units; `None` for a line without fill.
    pub width: Option<f32>,
    pub fill: FillSummary,
    pub cap: Option<String>,
    /// Dash and space lengths in stroke widths; empty for solid.
    pub dash: Vec<(f32, f32)>,
    pub head: Option<(String, String, String)>,
    pub tail: Option<(String, String, String)>,
}

/// One shape of a slide, with its frame in slide units and slide axes:
/// the transforms of its groups are applied.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeSummary {
    pub kind: ShapeKind,
    pub id: u32,
    pub name: String,
    pub frame: crate::document::Frame,
    pub geometry: Option<String>,
    pub adjust: Option<i64>,
    pub fill: Option<FillSummary>,
    pub line: Option<LineSummary>,
    pub children: Vec<ShapeSummary>,
}

impl ShapeSummary {
    /// The shapes of the tree, depth first, groups before their children.
    pub fn walk(&self) -> Vec<&ShapeSummary> {
        let mut out = vec![self];
        for child in &self.children {
            out.extend(child.walk());
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlideSummary {
    pub part: String,
    pub shapes: Vec<ShapeSummary>,
}

impl Deck {
    /// The slides with their shapes.
    pub fn summary(&self) -> Result<Vec<SlideSummary>, InspectError> {
        self.slides()?
            .into_iter()
            .map(|part| {
                let shapes = self.with_xml(&part, |document| {
                    let tree = document
                        .descendants()
                        .find(|node| node.has_tag_name((P_NS, "spTree")));
                    tree.map(|tree| {
                        let rels = self.rels(&part).unwrap_or_default();
                        shapes_of(tree, &Placement::default(), &rels)
                    })
                    .unwrap_or_default()
                })?;
                Ok(SlideSummary { part, shapes })
            })
            .collect()
    }
}

/// How the child units of the enclosing groups map to the slide.
#[derive(Clone, Default)]
struct Placement {
    /// From the innermost group out: child offset and extent, group box.
    groups: Vec<GroupTransform>,
}

#[derive(Clone)]
struct GroupTransform {
    child: [f32; 4],
    frame: crate::document::Frame,
}

impl Placement {
    /// A frame in the child units of the groups, to slide axes.
    fn to_slide(&self, mut frame: crate::document::Frame) -> crate::document::Frame {
        for group in self.groups.iter().rev() {
            let [cx, cy, cw, ch] = group.child;
            let sx = if cw > 0. { group.frame.width / cw } else { 1. };
            let sy = if ch > 0. { group.frame.height / ch } else { 1. };
            frame.x = group.frame.x + (frame.x - cx) * sx;
            frame.y = group.frame.y + (frame.y - cy) * sy;
            frame.width *= sx;
            frame.height *= sy;
            // The group turns its children around its own center.
            frame = crate::document::turn_frame(&frame, group.frame.center(), group.frame.rotation);
        }
        frame
    }
}

fn shapes_of(tree: Node, placement: &Placement, rels: &[Relationship]) -> Vec<ShapeSummary> {
    tree.children()
        .filter(Node::is_element)
        .filter_map(|node| shape_of(node, placement, rels))
        .collect()
}

fn shape_of(node: Node, placement: &Placement, rels: &[Relationship]) -> Option<ShapeSummary> {
    let kind = match node.tag_name().name() {
        "sp" => ShapeKind::Shape,
        "cxnSp" => ShapeKind::Connector,
        "grpSp" => ShapeKind::Group,
        "pic" => ShapeKind::Picture,
        "graphicFrame" => ShapeKind::Frame,
        _ => return None,
    };
    let names = node
        .descendants()
        .find(|child| child.has_tag_name((P_NS, "cNvPr")))?;
    let properties = node.children().find(|child| {
        matches!(child.tag_name().name(), "spPr" | "grpSpPr" | "xfrm")
            && child.tag_name().namespace() == Some(P_NS)
    });
    let xfrm = properties.and_then(|properties| {
        if properties.tag_name().name() == "xfrm" {
            Some(properties)
        } else {
            a(properties, "xfrm")
        }
    });
    let local = xfrm.map(read_xfrm).unwrap_or_default();
    let frame = placement.to_slide(local.frame);
    let mut summary = ShapeSummary {
        kind,
        id: names
            .attribute("id")
            .and_then(|id| id.parse().ok())
            .unwrap_or(0),
        name: names.attribute("name").unwrap_or_default().to_string(),
        frame,
        geometry: None,
        adjust: None,
        fill: None,
        line: None,
        children: Vec::new(),
    };
    if let Some(properties) = properties {
        if let Some(geometry) = a(properties, "prstGeom") {
            summary.geometry = geometry.attribute("prst").map(str::to_string);
            summary.adjust = geometry
                .descendants()
                .find(|gd| gd.has_tag_name((A_NS, "gd")))
                .and_then(|gd| gd.attribute("fmla"))
                .and_then(|fmla| fmla.strip_prefix("val "))
                .and_then(|value| value.trim().parse().ok());
        }
        summary.fill = fill_of(properties, rels);
        summary.line = a(properties, "ln").map(|line| line_of(line, rels));
    }
    if kind == ShapeKind::Picture
        && let Some(blip) = p(node, "blipFill")
    {
        summary.fill = blip_of(blip, rels);
    }
    if kind == ShapeKind::Group {
        let mut inner = placement.clone();
        inner.groups.push(GroupTransform {
            child: local.child,
            frame: local.frame,
        });
        // The group box itself is in the units of the outer groups.
        summary.children = shapes_of(node, &inner, rels);
    }
    Some(summary)
}

#[derive(Default)]
struct Xfrm {
    frame: crate::document::Frame,
    child: [f32; 4],
}

fn read_xfrm(xfrm: Node) -> Xfrm {
    let number = |node: Option<Node>, name: &str| {
        node.and_then(|node| node.attribute(name))
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0)
    };
    let off = a(xfrm, "off");
    let ext = a(xfrm, "ext");
    let rotation = xfrm
        .attribute("rot")
        .and_then(|value| value.parse::<i64>().ok())
        .map_or(0., super::units::degrees);
    let child_off = a(xfrm, "chOff");
    let child_ext = a(xfrm, "chExt");
    let u = super::units::units;
    Xfrm {
        frame: crate::document::Frame {
            x: u(number(off, "x")),
            y: u(number(off, "y")),
            width: u(number(ext, "cx")),
            height: u(number(ext, "cy")),
            rotation,
        },
        child: [
            u(number(child_off, "x")),
            u(number(child_off, "y")),
            u(number(child_ext, "cx")),
            u(number(child_ext, "cy")),
        ],
    }
}

fn color_of(node: Node) -> Option<Color> {
    let color = a(node, "srgbClr")?;
    let rgb = u32::from_str_radix(color.attribute("val")?, 16).ok()?;
    let alpha = a(color, "alpha")
        .and_then(|alpha| alpha.attribute("val"))
        .and_then(|value| value.parse::<i64>().ok())
        .map_or(1., super::units::fraction);
    Some(Color { rgb, alpha })
}

fn insets(node: Option<Node>) -> [f32; 4] {
    let value = |name| {
        node.and_then(|node| node.attribute(name))
            .and_then(|value| value.parse::<i64>().ok())
            .map_or(0., super::units::fraction)
    };
    [value("l"), value("t"), value("r"), value("b")]
}

fn stops_of(gradient: Node) -> Vec<(f32, Color)> {
    a(gradient, "gsLst")
        .map(|list| {
            list.children()
                .filter(|stop| stop.has_tag_name((A_NS, "gs")))
                .filter_map(|stop| {
                    let position = stop.attribute("pos")?.parse::<i64>().ok()?;
                    Some((super::units::fraction(position), color_of(stop)?))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The fill among the children of `properties` (an `spPr` or an `ln`).
fn fill_of(properties: Node, rels: &[Relationship]) -> Option<FillSummary> {
    for child in properties.children().filter(Node::is_element) {
        match child.tag_name().name() {
            "noFill" => return Some(FillSummary::None),
            "solidFill" => return color_of(child).map(FillSummary::Solid),
            "gradFill" => {
                let stops = stops_of(child);
                if let Some(linear) = a(child, "lin") {
                    return Some(FillSummary::Linear {
                        angle: linear
                            .attribute("ang")
                            .and_then(|value| value.parse::<i64>().ok())
                            .map_or(0., |angle| angle as f32 / 60_000.),
                        scaled: linear.attribute("scaled") == Some("1"),
                        stops,
                    });
                }
                let path = a(child, "path");
                return Some(FillSummary::Path {
                    path: path
                        .and_then(|path| path.attribute("path"))
                        .unwrap_or_default()
                        .to_string(),
                    fill_to: insets(path.and_then(|path| a(path, "fillToRect"))),
                    tile: insets(a(child, "tileRect")),
                    stops,
                });
            }
            "blipFill" => return blip_of(child, rels),
            _ => {}
        }
    }
    None
}

fn blip_of(blip_fill: Node, rels: &[Relationship]) -> Option<FillSummary> {
    let blip = a(blip_fill, "blip")?;
    let id = blip.attribute((R_NS, "embed"))?;
    let part = rels.iter().find(|rel| rel.id == id)?.target.clone();
    let alpha = a(blip, "alphaModFix")
        .and_then(|alpha| alpha.attribute("amt"))
        .and_then(|value| value.parse::<i64>().ok())
        .map_or(1., super::units::fraction);
    Some(FillSummary::Blip {
        part,
        alpha,
        source: insets(a(blip_fill, "srcRect")),
        fill: insets(a(blip_fill, "stretch").and_then(|stretch| a(stretch, "fillRect"))),
    })
}

fn line_of(line: Node, rels: &[Relationship]) -> LineSummary {
    let end = |name| {
        a(line, name).map(|end| {
            let value = |key| end.attribute(key).unwrap_or("med").to_string();
            (
                end.attribute("type").unwrap_or("none").to_string(),
                value("w"),
                value("len"),
            )
        })
    };
    let dash = a(line, "custDash")
        .map(|dash| {
            dash.children()
                .filter(|ds| ds.has_tag_name((A_NS, "ds")))
                .map(|ds| {
                    let value = |key| {
                        ds.attribute(key)
                            .and_then(|value| value.parse::<i64>().ok())
                            .map_or(0., super::units::fraction)
                    };
                    (value("d"), value("sp"))
                })
                .collect()
        })
        .unwrap_or_default();
    LineSummary {
        width: line
            .attribute("w")
            .and_then(|value| value.parse::<i64>().ok())
            .map(super::units::units),
        fill: fill_of(line, rels).unwrap_or(FillSummary::None),
        cap: line.attribute("cap").map(str::to_string),
        dash,
        head: end("headEnd"),
        tail: end("tailEnd"),
    }
}
