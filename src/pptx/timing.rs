//! `p:timing`: when the videos of a slide play. Fills that start `auto`
//! play when the slide shows, together; each `on_click` fill starts on its
//! own click, in layer order, before the next slide. This is the tree
//! PowerPoint writes for "Start: Automatically" and "Start: On Click".

use crate::document::Start;

use super::xml::Xml;

/// A video on the slide.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaNode {
    /// The `cNvPr` id of its picture.
    pub shape: u32,
    pub start: Start,
    pub looped: bool,
    pub muted: bool,
    /// Seconds.
    pub duration: f32,
}

/// Writes `p:timing` for `nodes`; nothing when there are none.
pub fn write(xml: &mut Xml, nodes: &[MediaNode]) {
    if nodes.is_empty() {
        return;
    }
    let mut ids = 0u32;
    let mut next_id = || {
        ids += 1;
        ids
    };
    xml.start("p:timing").start("p:tnLst").start("p:par");
    xml.start("p:cTn")
        .attr("id", next_id())
        .attr("dur", "indefinite")
        .attr("restart", "never")
        .attr("nodeType", "tmRoot")
        .start("p:childTnLst");

    // The main sequence of click groups.
    let sequence_id = next_id();
    xml.start("p:seq")
        .attr("concurrent", 1)
        .attr("nextAc", "seek")
        .start("p:cTn")
        .attr("id", sequence_id)
        .attr("dur", "indefinite")
        .attr("nodeType", "mainSeq")
        .start("p:childTnLst");
    let auto: Vec<&MediaNode> = nodes
        .iter()
        .filter(|node| node.start == Start::Auto)
        .collect();
    if !auto.is_empty() {
        group(xml, &mut next_id, Some(sequence_id), &auto);
    }
    for node in nodes.iter().filter(|node| node.start == Start::OnClick) {
        group(xml, &mut next_id, None, &[node]);
    }
    xml.end().end();
    xml.start("p:prevCondLst")
        .start("p:cond")
        .attr("evt", "onPrev")
        .attr("delay", 0)
        .start("p:tgtEl")
        .empty("p:sldTgt", &[])
        .end()
        .end()
        .end()
        .start("p:nextCondLst")
        .start("p:cond")
        .attr("evt", "onNext")
        .attr("delay", 0)
        .start("p:tgtEl")
        .empty("p:sldTgt", &[])
        .end()
        .end()
        .end();
    xml.end();

    // The media nodes: volume, mute and repeat of each video.
    for node in nodes {
        xml.start("p:video")
            .start("p:cMediaNode")
            .attr("vol", 80_000)
            .attr_opt("mute", node.muted.then_some(1));
        xml.start("p:cTn")
            .attr("id", next_id())
            .attr_opt("repeatCount", node.looped.then_some("indefinite"))
            .attr("fill", "hold")
            .attr("display", 0)
            .start("p:stCondLst")
            .empty("p:cond", &[("delay", &"indefinite")])
            .end()
            .end();
        target(xml, node.shape);
        xml.end().end();
    }

    xml.end().end().end().end().end();
}

/// One click group of the main sequence. `with_slide` makes the group start
/// with the slide (the id of the main sequence) instead of on a click.
fn group(
    xml: &mut Xml,
    next_id: &mut impl FnMut() -> u32,
    with_slide: Option<u32>,
    nodes: &[&MediaNode],
) {
    xml.start("p:par")
        .start("p:cTn")
        .attr("id", next_id())
        .attr("fill", "hold")
        .start("p:stCondLst")
        .empty("p:cond", &[("delay", &"indefinite")]);
    if let Some(sequence) = with_slide {
        xml.start("p:cond")
            .attr("evt", "onBegin")
            .attr("delay", 0)
            .empty("p:tn", &[("val", &sequence)])
            .end();
    }
    xml.end().start("p:childTnLst");
    xml.start("p:par")
        .start("p:cTn")
        .attr("id", next_id())
        .attr("fill", "hold")
        .start("p:stCondLst")
        .empty("p:cond", &[("delay", &0)])
        .end()
        .start("p:childTnLst");
    for (index, node) in nodes.iter().enumerate() {
        let kind = match (with_slide, index) {
            (None, _) => "clickEffect",
            (Some(_), 0) => "afterEffect",
            (Some(_), _) => "withEffect",
        };
        xml.start("p:par")
            .start("p:cTn")
            .attr("id", next_id())
            .attr("presetID", 1)
            .attr("presetClass", "mediacall")
            .attr("presetSubtype", 0)
            .attr("fill", "hold")
            .attr("nodeType", kind)
            .start("p:stCondLst")
            .empty("p:cond", &[("delay", &0)])
            .end()
            .start("p:childTnLst")
            .start("p:cmd")
            .attr("type", "call")
            .attr("cmd", "playFrom(0.0)")
            .start("p:cBhvr")
            .empty(
                "p:cTn",
                &[
                    ("id", &next_id()),
                    ("dur", &((node.duration * 1000.).round().max(1.) as u32)),
                    ("fill", &"hold"),
                ],
            );
        target(xml, node.shape);
        xml.end().end().end().end().end();
    }
    xml.end().end().end();
    xml.end().end().end();
}

fn target(xml: &mut Xml, shape: u32) {
    xml.start("p:tgtEl")
        .empty("p:spTgt", &[("spid", &shape)])
        .end();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(shape: u32, start: Start) -> MediaNode {
        MediaNode {
            shape,
            start,
            looped: true,
            muted: false,
            duration: 2.5,
        }
    }

    fn written(nodes: &[MediaNode]) -> String {
        let mut xml = Xml::fragment();
        write(&mut xml, nodes);
        xml.finish()
    }

    #[test]
    fn no_videos_no_timing() {
        assert_eq!(written(&[]), "");
    }

    #[test]
    fn automatic_videos_start_with_the_slide_and_clicks_follow() {
        let xml = written(&[
            node(4, Start::OnClick),
            node(2, Start::Auto),
            node(3, Start::Auto),
        ]);
        assert_eq!(xml.matches(r#"evt="onBegin""#).count(), 1);
        let after = xml.find(r#"nodeType="afterEffect""#).unwrap();
        let with = xml.find(r#"nodeType="withEffect""#).unwrap();
        let click = xml.find(r#"nodeType="clickEffect""#).unwrap();
        assert!(after < with && with < click);
        assert_eq!(xml.matches("<p:video>").count(), 3);
        assert_eq!(xml.matches(r#"dur="2500""#).count(), 3);
        assert_eq!(xml.matches(r#"repeatCount="indefinite""#).count(), 3);
    }

    #[test]
    fn ids_are_unique() {
        let xml = written(&[node(2, Start::Auto), node(3, Start::OnClick)]);
        let wrapped = format!(r#"<r xmlns:p="{}">{xml}</r>"#, super::super::P_NS);
        let document = roxmltree::Document::parse(&wrapped).unwrap();
        let mut ids: Vec<&str> = document
            .descendants()
            .filter(|node| node.has_tag_name((super::super::P_NS, "cTn")))
            .filter_map(|node| node.attribute("id"))
            .collect();
        let count = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
