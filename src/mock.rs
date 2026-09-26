//! Static data shown by the editor shell until a document model exists.

pub struct Document {
    pub folder: &'static str,
    pub title: &'static str,
    pub extension: &'static str,
}

pub const DOCUMENT: Document = Document {
    folder: "Drafts",
    title: "Q3 Product Review",
    extension: ".sldr",
};
