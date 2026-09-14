pub mod config;
pub mod epub;
pub mod image;
pub mod input;
pub(crate) mod jis;
pub mod metadata;
pub mod pipeline;
pub mod text;

pub use config::{AozoraConfig, ConfigError, IniSettings, StyleSettings, SuffixNoteRule};
pub use epub::{EpubAsset, EpubBook, EpubError, EpubMetadata, EpubSection, NavChapter};
pub use input::{
    FileSource, Input, InputError, TextEntry, decode_text, detect_encoding, normalize_entry_path,
};
pub use metadata::{
    BookMeta, TitleType, detect_meta, detect_meta_with_gaiji, file_title_creator,
    remove_metadata_lines,
};
pub use pipeline::{
    CollectedAsset, ImageDimensions, ImagePageFit, ImagePageType, append_gaiji_assets,
    build_metadata, build_title_page_markup, collect_assets, decorate_image_tags, image_dimensions,
    is_auto_cover, is_no_cover, is_same_name_cover, java_name_uuid, reflow_image_sections,
    remove_image_sources, remove_missing_image_sources, rewrite_image_source,
    split_image_page_sections, svg_image_fragment,
};
pub use text::{
    ChapterRecord, TextError, aozora_text_to_xhtml_sections,
    aozora_text_to_xhtml_sections_with_chapters, aozora_text_to_xhtml_sections_with_config,
    apply_alt_upright, collect_image_alts, decode_input, escape_html, image_reference_occurrences,
    image_references, inline_to_xhtml, plain_text_to_xhtml, plain_text_to_xhtml_with_config,
    tcy_label,
};
