pub mod config;
pub mod epub;
pub mod text;

pub use config::{AozoraConfig, ConfigError, IniSettings, SuffixNoteRule};
pub use epub::{EpubAsset, EpubBook, EpubError, EpubMetadata, EpubSection};
pub use text::{
    TextError, aozora_text_to_xhtml_sections, aozora_text_to_xhtml_sections_with_config,
    decode_input, escape_html, image_references, plain_text_to_xhtml,
    plain_text_to_xhtml_with_config,
};
