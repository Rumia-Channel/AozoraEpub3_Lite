pub mod config;
pub mod epub;
pub mod image;
pub mod input;
pub mod metadata;
pub mod text;

pub use config::{AozoraConfig, ConfigError, IniSettings, SuffixNoteRule};
pub use epub::{EpubAsset, EpubBook, EpubError, EpubMetadata, EpubSection};
pub use input::{Input, InputError, TextEntry, decode_text, detect_encoding, normalize_entry_path};
pub use metadata::{BookMeta, TitleType, detect_meta, file_title_creator};
pub use text::{
    TextError, aozora_text_to_xhtml_sections, aozora_text_to_xhtml_sections_with_config,
    decode_input, escape_html, image_reference_occurrences, image_references, plain_text_to_xhtml,
    plain_text_to_xhtml_with_config,
};
