//! One slide part: the shape tree with the elements of the slide, and the
//! relationships to the layout and to the media the shapes use.

use crate::document::{Arrowhead, Element, ElementKind, Frame, Presentation, Slide, turn_frame};

use super::package::{Package, Rels, rel};
use super::xml::Xml;
use super::{A_NS, Options, P_NS, R_NS, Warning};
use super::{shapes, skeleton};

/// A slide part and its relationships.
pub struct Written {
    pub xml: String,
    pub rels: Rels,
}

/// The media parts of the deck, shared by all slides: each image or video
/// is written once.
#[derive(Default)]
pub struct Media {
    parts: Vec<(String, Vec<u8>)>,
}

impl Media {
    pub fn write(self, package: &mut Package) {
        for (name, bytes) in self.parts {
            package.binary(name, bytes, false);
        }
    }
}

/// The shapes of one slide, with the ids and relationships they need.
pub struct SlideWriter<'a> {
    pub slide: &'a Slide,
    pub xml: Xml,
    pub rels: Rels,
    pub warnings: &'a mut Vec<Warning>,
    /// The next `cNvPr` id. Id 1 is the shape tree.
    next_id: u32,
}

/// A turn of the coordinates around a pivot, in degrees. The children of
/// a rotated group are written in the unrotated axes of the group.
type Turn = ((f32, f32), f32);

/// The frame of an element in the axes of its group.
fn placed(frame: &Frame, turns: &[Turn]) -> Frame {
    turns.iter().fold(*frame, |frame, (pivot, degrees)| {
        turn_frame(&frame, *pivot, *degrees)
    })
}

impl SlideWriter<'_> {
    /// A new shape id, unique on the slide.
    pub fn shape_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn warn(&mut self, element: &Element, message: impl Into<String>) {
        self.warnings.push(Warning {
            slide: Some(self.slide.id),
            element: Some(element.id),
            message: message.into(),
        });
    }

    /// `p:cNvPr`: the shape id and the element name.
    fn names(&mut self, element: &Element, kind: &str) {
        let id = self.shape_id();
        let name = element
            .name
            .clone()
            .unwrap_or_else(|| format!("{kind} {}", element.id.0));
        self.xml.empty("p:cNvPr", &[("id", &id), ("name", &name)]);
    }

    /// Writes an element and its children. `opacity` is the product of the
    /// opacities of its groups.
    pub fn element(&mut self, element: &Element, opacity: f32, turns: &[Turn]) {
        if element.hidden {
            return;
        }
        let opacity = opacity * element.opacity;
        let frame = placed(&element.frame, turns);
        match &element.kind {
            ElementKind::Group(group) => {
                self.xml.start("p:grpSp").start("p:nvGrpSpPr");
                self.names(element, "Group");
                self.xml
                    .empty("p:cNvGrpSpPr", &[])
                    .empty("p:nvPr", &[])
                    .end()
                    .start("p:grpSpPr");
                shapes::group_transform(&mut self.xml, &frame);
                self.xml.end();
                let mut inner = turns.to_vec();
                if frame.rotation != 0. {
                    inner.push((frame.center(), -frame.rotation));
                }
                for child in &group.children {
                    self.element(child, opacity, &inner);
                }
                self.xml.end();
            }
            ElementKind::Rectangle(shape) => {
                let adjust = shapes::corner_adjust(shape.corner_radius, frame.width, frame.height);
                let preset = if adjust > 0 { "roundRect" } else { "rect" };
                self.shape(
                    element,
                    &frame,
                    "Rectangle",
                    preset,
                    (adjust > 0).then_some(adjust),
                    opacity,
                );
            }
            ElementKind::Ellipse(_) => {
                self.shape(element, &frame, "Ellipse", "ellipse", None, opacity);
            }
            ElementKind::Line(line) => {
                self.xml.start("p:cxnSp").start("p:nvCxnSpPr");
                self.names(element, "Line");
                self.xml
                    .empty("p:cNvCxnSpPr", &[])
                    .empty("p:nvPr", &[])
                    .end()
                    .start("p:spPr");
                shapes::line_transform(&mut self.xml, &frame);
                shapes::geometry(&mut self.xml, "line", None);
                shapes::outline(
                    &mut self.xml,
                    Some(&line.stroke),
                    opacity,
                    [line.start, line.end],
                );
                self.xml.end().end();
            }
            ElementKind::Text(_) | ElementKind::Table(_) => {
                self.warn(element, "not exported yet");
            }
        }
    }

    /// A rectangle or an ellipse.
    fn shape(
        &mut self,
        element: &Element,
        frame: &Frame,
        kind: &str,
        preset: &str,
        adjust: Option<i64>,
        opacity: f32,
    ) {
        self.xml.start("p:sp").start("p:nvSpPr");
        self.names(element, kind);
        self.xml
            .empty("p:cNvSpPr", &[])
            .empty("p:nvPr", &[])
            .end()
            .start("p:spPr");
        shapes::transform(&mut self.xml, frame);
        shapes::geometry(&mut self.xml, preset, adjust);
        if let Some(fill) = element.kind.fill() {
            shapes::fill(&mut self.xml, fill, opacity, frame.width, frame.height);
        }
        shapes::outline(
            &mut self.xml,
            element.kind.stroke(),
            opacity,
            [Arrowhead::NONE; 2],
        );
        self.xml.end().end();
    }
}

pub fn write(
    presentation: &Presentation,
    slide: &Slide,
    options: &Options,
    media: &mut Media,
    warnings: &mut Vec<Warning>,
) -> Written {
    let mut rels = Rels::default();
    rels.add(rel::SLIDE_LAYOUT, "../slideLayouts/slideLayout1.xml");
    let mut xml = Xml::new();
    xml.start("p:sld")
        .attr("xmlns:a", A_NS)
        .attr("xmlns:r", R_NS)
        .attr("xmlns:p", P_NS);
    xml.start("p:cSld")
        .start("p:spTree")
        .start("p:nvGrpSpPr")
        .empty("p:cNvPr", &[("id", &1), ("name", &"")])
        .empty("p:cNvGrpSpPr", &[])
        .empty("p:nvPr", &[])
        .end();
    skeleton::group_properties(&mut xml);
    let _ = (presentation, options, media);
    let mut writer = SlideWriter {
        slide,
        xml,
        rels,
        warnings,
        next_id: 2,
    };
    for element in &slide.elements {
        writer.element(element, 1., &[]);
    }
    let SlideWriter { mut xml, rels, .. } = writer;
    xml.end().end();
    xml.start("p:clrMapOvr")
        .empty("a:masterClrMapping", &[])
        .end();
    xml.end();
    Written {
        xml: xml.finish(),
        rels,
    }
}
