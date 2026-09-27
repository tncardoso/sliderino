//! One slide part: the shape tree with the elements of the slide, and the
//! relationships to the layout and to the media the shapes use.

use crate::document::{
    Arrowhead, Dash, Element, ElementKind, Fill, Frame, HAlign, ImageFill, Presentation, Rgb,
    Slide, Stroke, TextElement, TextSizing, VAlign, rotate_vector, turn_frame,
};
use crate::table::TableElement;
use crate::text_layout;

use super::units::emu;

use super::media::Media;
use super::package::{Rels, rel};
use super::xml::Xml;
use super::{A_NS, Options, P_NS, R_NS, Warning};
use super::{shapes, skeleton};

/// A slide part and its relationships.
pub struct Written {
    pub xml: String,
    pub rels: Rels,
}

/// The shapes of one slide, with the ids and relationships they need.
pub struct SlideWriter<'a> {
    pub presentation: &'a Presentation,
    pub slide: &'a Slide,
    pub options: &'a Options,
    pub xml: Xml,
    pub rels: Rels,
    pub media: &'a mut Media,
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
            ElementKind::Text(text) => self.text(element, text, &frame, opacity),
            ElementKind::Table(table) => self.table(element, table, &frame, opacity, turns),
        }
    }

    /// A table. PowerPoint does not turn tables, so a turned table is a
    /// group of its fills, borders and texts.
    fn table(
        &mut self,
        element: &Element,
        table: &TableElement,
        frame: &Frame,
        opacity: f32,
        turns: &[Turn],
    ) {
        let Ok(layout) = crate::table::layout(table, &self.presentation.fonts) else {
            self.warn(element, "a font of the table cannot be read");
            return;
        };
        if layout.missing_glyphs > 0 {
            self.warn(
                element,
                format!(
                    "the fonts have no glyph for {} character(s)",
                    layout.missing_glyphs
                ),
            );
        }
        if frame.rotation != 0. {
            self.warn(
                element,
                "PowerPoint cannot turn a table: it is a group of shapes",
            );
            let parts = crate::table::parts(element.id, &element.frame, table, &layout);
            let group = Element {
                name: Some(
                    element
                        .name
                        .clone()
                        .unwrap_or_else(|| format!("Table {}", element.id.0)),
                ),
                opacity: 1.,
                hidden: false,
                ..Element::new(
                    element.id,
                    element.frame,
                    ElementKind::Group(crate::document::GroupElement {
                        children: parts.iter().map(|part| part.element().clone()).collect(),
                    }),
                )
            };
            // `opacity` already holds the opacity of the table.
            self.element(&group, opacity, turns);
            return;
        }

        let mut cells = Vec::with_capacity(layout.cells.len());
        let owners = table.owners();
        for cell in &layout.cells {
            let Some(font) = self.presentation.fonts.get(&cell.text.style.font) else {
                self.warn(element, "a font of the table is not in the presentation");
                return;
            };
            let shift = super::text::baseline_shift(&cell.text.style, font);
            cells.push(super::table::CellSpec {
                text: &cell.text,
                layout: &cell.layout,
                font,
                fill: table.fill_of(cell.row, cell.column),
                borders: super::table::borders(table, &owners, cell.row, cell.column),
                margins: super::table::margins(table.padding, &cell.text, shift),
            });
        }
        let mut graphic = Xml::fragment();
        let presentation = self.presentation;
        let media = &mut *self.media;
        let rels = &mut self.rels;
        let mut problems = Vec::new();
        super::table::graphic(
            &mut graphic,
            table,
            &layout,
            &cells,
            opacity,
            |xml, fill, size| {
                let Fill::Image(image) = fill else {
                    return false;
                };
                let Some(data) = presentation.images.get(image.id) else {
                    problems.push(format!("image {} is not in the presentation", image.id.0));
                    return false;
                };
                match media.image(presentation, image.id) {
                    Ok(target) => {
                        let rel = rels.get_or_add(rel::IMAGE, &target);
                        shapes::blip_fill(
                            xml,
                            &rel,
                            image.fit,
                            size,
                            (data.width, data.height),
                            image.opacity * opacity,
                        );
                        true
                    }
                    Err(message) => {
                        problems.push(message);
                        false
                    }
                }
            },
        );
        for problem in problems {
            self.warn(element, problem);
        }

        let table_frame = Frame {
            width: layout.width(),
            height: layout.height(),
            ..*frame
        };
        self.xml.start("p:graphicFrame").start("p:nvGraphicFramePr");
        self.names(element, "Table");
        self.xml
            .start("p:cNvGraphicFramePr")
            .empty("a:graphicFrameLocks", &[("noGrp", &1)])
            .end()
            .empty("p:nvPr", &[])
            .end()
            .start("p:xfrm")
            .empty(
                "a:off",
                &[("x", &emu(table_frame.x)), ("y", &emu(table_frame.y))],
            )
            .empty(
                "a:ext",
                &[
                    ("cx", &emu(table_frame.width)),
                    ("cy", &emu(table_frame.height)),
                ],
            )
            .end()
            .raw(&graphic.finish())
            .end();
    }

    /// The `a:blipFill` of an image fill; no fill when the image is missing.
    fn image_fill(&mut self, element: &Element, image: &ImageFill, frame: &Frame, opacity: f32) {
        let Some(data) = self.presentation.images.get(image.id) else {
            self.warn(
                element,
                format!("image {} is not in the presentation", image.id.0),
            );
            self.xml.empty("a:noFill", &[]);
            return;
        };
        let size = (data.width, data.height);
        match self.media.image(self.presentation, image.id) {
            Ok(target) => {
                let rel = self.rels.get_or_add(rel::IMAGE, &target);
                shapes::blip_fill(
                    &mut self.xml,
                    &rel,
                    image.fit,
                    (frame.width, frame.height),
                    size,
                    image.opacity * opacity,
                );
            }
            Err(message) => {
                self.warn(element, message);
                self.xml.empty("a:noFill", &[]);
            }
        }
    }

    /// A text box. The box moves by the baseline shift, so the viewer puts
    /// the lines where Sliderino does.
    fn text(&mut self, element: &Element, text: &TextElement, frame: &Frame, opacity: f32) {
        let Some(font) = self.presentation.fonts.get(&text.style.font) else {
            self.warn(element, "its font is not in the presentation");
            return;
        };
        let Ok(layout) = text_layout::layout(text, &element.frame, font) else {
            self.warn(element, "its font cannot be read");
            return;
        };
        if layout.overflow() > 0. {
            self.warn(element, "the text runs past the bottom of its box");
        }
        if layout.missing_glyphs > 0 {
            self.warn(
                element,
                format!(
                    "the font has no glyph for {} character(s)",
                    layout.missing_glyphs
                ),
            );
        }
        let shift = super::text::baseline_shift(&text.style, font);
        let (dx, dy) = rotate_vector(0., -shift, frame.rotation);
        let moved = Frame {
            x: frame.x + dx,
            y: frame.y + dy,
            ..*frame
        };
        let anchor = match text.sizing {
            TextSizing::Fixed => text.style.vertical_align,
            _ => VAlign::Top,
        };
        let justify = text.style.align == HAlign::Justify;

        self.xml.start("p:sp").start("p:nvSpPr");
        self.names(element, "Text");
        self.xml
            .empty("p:cNvSpPr", &[("txBox", &1)])
            .empty("p:nvPr", &[])
            .end()
            .start("p:spPr");
        shapes::transform(&mut self.xml, &moved);
        shapes::geometry(&mut self.xml, "rect", None);
        self.xml.empty("a:noFill", &[]).end().start("p:txBody");
        super::text::body_properties(&mut self.xml, anchor, justify, [0.; 4]);
        self.xml.empty("a:lstStyle", &[]);
        super::text::paragraphs(
            &mut self.xml,
            &text.content,
            &text.style,
            &layout,
            font,
            opacity,
        );
        self.xml.end().end();

        if self.options.guides {
            self.guides(frame, &layout);
        }
    }

    /// Thin lines on the frame and on the baselines of the layout, to see
    /// where a viewer puts the text.
    fn guides(&mut self, frame: &Frame, layout: &text_layout::TextLayout) {
        let color = Rgb(0x00B7EB);
        let stroke = Stroke {
            color,
            opacity: 1.,
            width: 1.,
            dash: Dash::Solid,
        };
        let edges = [
            (0., 0., frame.width, 0.),
            (0., frame.height, frame.width, frame.height),
        ];
        let baselines = layout
            .lines
            .iter()
            .filter(|line| line.right > line.left)
            .map(|line| (line.left, line.baseline, line.right, line.baseline));
        for (left, y, right, _) in edges.into_iter().chain(baselines) {
            let local = Frame {
                x: frame.x + left,
                y: frame.y + y,
                width: right - left,
                height: 0.,
                rotation: 0.,
            };
            let line = turn_frame(&local, frame.center(), frame.rotation);
            let id = self.shape_id();
            self.xml
                .start("p:cxnSp")
                .start("p:nvCxnSpPr")
                .empty("p:cNvPr", &[("id", &id), ("name", &format!("Guide {id}"))])
                .empty("p:cNvCxnSpPr", &[])
                .empty("p:nvPr", &[])
                .end()
                .start("p:spPr");
            shapes::line_transform(&mut self.xml, &line);
            shapes::geometry(&mut self.xml, "line", None);
            shapes::outline(&mut self.xml, Some(&stroke), 1., [Arrowhead::NONE; 2]);
            self.xml.end().end();
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
        match element.kind.fill() {
            Some(Fill::Image(image)) => self.image_fill(element, image, frame, opacity),
            Some(fill) => shapes::fill(&mut self.xml, fill, opacity, frame.width, frame.height),
            None => {}
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
    let mut writer = SlideWriter {
        presentation,
        slide,
        options,
        xml,
        rels,
        media,
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
