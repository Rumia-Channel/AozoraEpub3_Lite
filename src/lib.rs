pub mod epub;
pub mod text;

pub use epub::{EpubBook, EpubError, EpubMetadata, EpubSection};
pub use text::{
    TextError, aozora_text_to_xhtml_sections, decode_input, escape_html, plain_text_to_xhtml,
};
