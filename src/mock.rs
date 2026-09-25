//! Static data shown by the editor shell until a document model exists.

pub struct Document {
    pub folder: &'static str,
    pub title: &'static str,
    pub extension: &'static str,
    pub zoom: &'static str,
    pub agent_status: &'static str,
}

pub const DOCUMENT: Document = Document {
    folder: "Drafts",
    title: "Q3 Product Review",
    extension: ".sldr",
    zoom: "100%",
    agent_status: "Agent connected · MCP",
};

/// Arrangement drawn by a slide thumbnail.
pub enum SlideKind {
    Title {
        title: &'static str,
        subtitle: &'static str,
    },
    Kpi {
        title: &'static str,
        values: [&'static str; 3],
    },
    TwoColumn {
        title: &'static str,
    },
    ImageRight {
        title: &'static str,
    },
    Closing {
        title: &'static str,
    },
}

pub const SLIDES: [SlideKind; 5] = [
    SlideKind::Title {
        title: "Q3 Product Review",
        subtitle: "September 2026",
    },
    SlideKind::Kpi {
        title: "Activation grew faster than signups",
        values: ["+38%", "12.4k", "4.2m"],
    },
    SlideKind::TwoColumn {
        title: "What we shipped",
    },
    SlideKind::ImageRight {
        title: "The new canvas",
    },
    SlideKind::Closing {
        title: "Questions?",
    },
];

pub struct Selection {
    pub name: &'static str,
    pub origin: &'static str,
    pub x: &'static str,
    pub y: &'static str,
    pub width: &'static str,
    pub height: &'static str,
    pub rotation: &'static str,
    pub radius: &'static str,
    pub fill: &'static str,
    pub fill_opacity: &'static str,
}

pub const SELECTION: Selection = Selection {
    name: "KPI Box",
    origin: "Independent copy from library",
    x: "56",
    y: "218",
    width: "216",
    height: "148",
    rotation: "0°",
    radius: "6",
    fill: "F4F4F2",
    fill_opacity: "100%",
};

pub enum Severity {
    Ok,
    Warning,
}

pub struct Diagnostic {
    pub severity: Severity,
    pub message: &'static str,
}

pub const DIAGNOSTICS: [Diagnostic; 2] = [
    Diagnostic {
        severity: Severity::Ok,
        message: "All text fits its box",
    },
    Diagnostic {
        severity: Severity::Warning,
        message: "Label wraps to 2 lines in “4.2m”",
    },
];
