use std::fmt;

use crate::config::AozoraConfig;
use encoding_rs::{Encoding, SHIFT_JIS, UTF_8};
#[path = "text_inline.rs"]
mod inline;

use inline::convert_inline;
pub fn inline_to_xhtml(input: &str, config: &AozoraConfig) -> String {
    convert_inline(input, config)
}
pub use inline::{escape_html, image_reference_occurrences, image_references};

#[derive(Debug, Eq, PartialEq)]
pub enum TextError {
    InvalidInput,
    UnsupportedEncoding(String),
    DecodeError(String),
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput => write!(f, "input text is not valid UTF-8"),
            Self::UnsupportedEncoding(encoding) => {
                write!(f, "unsupported input encoding: {encoding}")
            }
            Self::DecodeError(encoding) => {
                write!(f, "input cannot be decoded as {encoding}")
            }
        }
    }
}

impl std::error::Error for TextError {}

pub fn decode_input(bytes: &[u8], label: Option<&str>) -> Result<String, TextError> {
    let encoding = match label {
        Some(label) => Encoding::for_label(label.as_bytes())
            .ok_or_else(|| TextError::UnsupportedEncoding(label.to_owned()))?,
        None => {
            let (_, _, had_errors) = UTF_8.decode(bytes);
            if had_errors { SHIFT_JIS } else { UTF_8 }
        }
    };
    let (decoded, _, had_errors) = encoding.decode(bytes);
    if had_errors {
        return Err(TextError::DecodeError(encoding.name().to_owned()));
    }
    Ok(decoded
        .strip_prefix('\u{feff}')
        .unwrap_or(decoded.as_ref())
        .to_owned())
}

pub fn plain_text_to_xhtml(input: &str) -> Result<String, TextError> {
    plain_text_to_xhtml_with_config(input, &AozoraConfig::default())
}

pub fn plain_text_to_xhtml_with_config(
    input: &str,
    config: &AozoraConfig,
) -> Result<String, TextError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let lines = visible_lines(input, config);
    Ok(render_lines(
        lines.iter().map(String::as_str),
        &[],
        None,
        config,
    ))
}

fn visible_lines(input: &str, config: &AozoraConfig) -> Vec<String> {
    let mut in_comment = false;
    let mut lines = input
        .lines()
        .filter_map(|line| {
            if is_comment_line(line) {
                in_comment = !in_comment;
                return config.comment_print.then(|| line.to_owned());
            }
            if in_comment {
                if !config.comment_print {
                    return None;
                }
                if !config.comment_convert {
                    return Some(format!("{RAW_COMMENT_PREFIX}{}", escape_comment_line(line)));
                }
            }
            Some(line.to_owned())
        })
        .collect::<Vec<_>>();
    if config.force_indent {
        for line in &mut lines {
            force_indent_line(line);
        }
    }
    normalize_empty_lines(lines, config)
}

fn force_indent_line(line: &mut String) {
    if line.starts_with(RAW_COMMENT_PREFIX) {
        return;
    }
    let mut chars = line.chars();
    let Some(first) = chars.next() else {
        return;
    };
    let Some(second) = chars.next() else {
        return;
    };
    if matches!(
        first,
        '\u{3000}' | '「' | '『' | '（' | '”' | '〈' | '【' | '〔' | '［' | '※'
    ) {
        return;
    }
    if first == ' ' || first == '\u{2000}' {
        let replacement_start = if second == ' ' || second == '\u{2000}' || second == '\u{3000}' {
            first.len_utf8()
        } else {
            0
        };
        let replacement_end = replacement_start
            + line[replacement_start..]
                .chars()
                .next()
                .map_or(0, char::len_utf8);
        line.replace_range(replacement_start..replacement_end, "\u{3000}");
    } else {
        line.insert(0, '\u{3000}');
    }
}

fn normalize_empty_lines(lines: Vec<String>, config: &AozoraConfig) -> Vec<String> {
    if config.remove_empty_line == 0 && config.max_empty_line == 0 {
        return lines;
    }
    let mut output = Vec::with_capacity(lines.len());
    let mut empty_count = 0usize;
    let mut after_heading = false;
    for line in lines {
        let is_empty = line.trim().is_empty()
            || (config.remove_empty_line > 0 && line.chars().all(char::is_whitespace));
        if is_empty {
            empty_count += 1;
            continue;
        }
        append_empty_lines(&mut output, empty_count, config, after_heading);
        empty_count = 0;
        after_heading = line.contains("見出し");
        output.push(line);
    }
    append_empty_lines(&mut output, empty_count, config, after_heading);
    output
}

fn append_empty_lines(
    output: &mut Vec<String>,
    empty_count: usize,
    config: &AozoraConfig,
    after_heading: bool,
) {
    if empty_count == 0 {
        return;
    }
    let mut keep = empty_count.saturating_sub(config.remove_empty_line);
    if config.max_empty_line > 0 {
        keep = keep.min(config.max_empty_line);
    }
    if after_heading && keep == 0 {
        keep = 1;
    }
    output.extend(std::iter::repeat_with(String::new).take(keep));
}

fn escape_comment_line(line: &str) -> String {
    line.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn is_comment_line(line: &str) -> bool {
    line.starts_with("--------------------------------------------------")
}

/// A navigation chapter detected during section splitting, mirroring the
/// reference converter's pre-read chapter model (TYPE_PAGEBREAK).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChapterRecord {
    /// Index of the body section (0-based) this chapter belongs to.
    pub section_index: usize,
    /// Index of the chapter line within the section's trimmed line list.
    pub line_index: usize,
    /// Normalized navigation label.
    pub label: String,
    /// 1-based heading level (1 for page-break chapters).
    pub level: u8,
}

pub fn aozora_text_to_xhtml_sections(input: &str) -> Result<Vec<String>, TextError> {
    aozora_text_to_xhtml_sections_with_config(input, &AozoraConfig::default())
}

pub fn aozora_text_to_xhtml_sections_with_config(
    input: &str,
    config: &AozoraConfig,
) -> Result<Vec<String>, TextError> {
    Ok(aozora_text_to_xhtml_sections_with_chapters(input, config, true)?.0)
}

/// Splits the input into body sections and detects navigation chapters the
/// way the reference converter's pre-read does: a chapter is the first
/// non-symbol line of each section (after a page break), with the whole line
/// counting even when the page-break note sits mid-line.
///
/// `initial_add_section_chapter` mirrors the pre-read state after the title
/// line: `false` when the metadata block already consumed the first-chapter
/// slot (so `input` should be the meta-stripped body in that case).
pub fn aozora_text_to_xhtml_sections_with_chapters(
    input: &str,
    config: &AozoraConfig,
    initial_add_section_chapter: bool,
) -> Result<(Vec<String>, Vec<ChapterRecord>), TextError> {
    let mut sections = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut no_br: Vec<bool> = Vec::new();
    let mut page_marker = None;
    let mut section_index = 0usize;
    let mut add_section_chapter = initial_add_section_chapter;
    let mut chapters: Vec<ChapterRecord> = Vec::new();
    let mut chapter_line: Option<usize> = None;

    for line in visible_lines(input, config).iter() {
        let line_start_section = section_index;
        let has_page_break = find_page_break_note(line, config).is_some();
        if has_page_break && config.split_page_breaks {
            add_section_chapter = true;
        }
        if add_section_chapter {
            let name = chapter_name(line, config);
            if !name.is_empty() && !symbols_only(&name) && !is_colophon_line(line) {
                chapters.push(ChapterRecord {
                    section_index: line_start_section,
                    line_index: current.len(),
                    label: name,
                    level: 1,
                });
                chapter_line = Some(current.len());
                add_section_chapter = false;
            } else if is_colophon_line(line) {
                add_section_chapter = false;
            }
        }

        let mut remainder = line.as_str();
        loop {
            let Some((offset, end, note)) = find_page_break_note(remainder, config) else {
                append_section_line(
                    remainder,
                    &mut sections,
                    &mut current,
                    &mut no_br,
                    &mut page_marker,
                    &mut section_index,
                    &mut chapter_line,
                    config,
                    false,
                );
                break;
            };

            if !remainder[..offset].is_empty() {
                append_section_line(
                    &remainder[..offset],
                    &mut sections,
                    &mut current,
                    &mut no_br,
                    &mut page_marker,
                    &mut section_index,
                    &mut chapter_line,
                    config,
                    true,
                );
            }
            if config.split_page_breaks {
                trim_trailing_empty_lines(&mut current);
                no_br.truncate(current.len());
                if !current.is_empty() {
                    push_rendered_section(
                        &mut sections,
                        &mut current,
                        &mut no_br,
                        &mut chapter_line,
                        page_marker,
                        config,
                    );
                    section_index += 1;
                }
                page_marker = if config.page_middle_notes.contains(&note) {
                    Some(PAGE_CHAPTER_MIDDLE_MARKER)
                } else if config.page_bottom_notes.contains(&note) {
                    Some(PAGE_CHAPTER_BOTTOM_MARKER)
                } else {
                    Some(PAGE_CHAPTER_MARKER)
                };
            }
            remainder = &remainder[end..];
            if remainder.is_empty() {
                break;
            }
        }
    }

    trim_trailing_empty_lines(&mut current);
    no_br.truncate(current.len());
    if !current.is_empty() || sections.is_empty() {
        push_rendered_section(
            &mut sections,
            &mut current,
            &mut no_br,
            &mut chapter_line,
            page_marker,
            config,
        );
    }

    Ok((sections, chapters))
}

fn push_rendered_section(
    sections: &mut Vec<String>,
    current: &mut Vec<String>,
    no_br: &mut Vec<bool>,
    chapter_line: &mut Option<usize>,
    page_marker: Option<&'static str>,
    config: &AozoraConfig,
) {
    let no_br_flags = no_br.clone();
    let fragment = render_marked_lines(
        current.iter().map(String::as_str),
        &no_br_flags,
        chapter_line.take(),
        config,
        page_marker,
    );
    sections.push(fragment);
    current.clear();
    no_br.clear();
}

fn append_section_line(
    line: &str,
    sections: &mut Vec<String>,
    current: &mut Vec<String>,
    no_br: &mut Vec<bool>,
    page_marker: &mut Option<&'static str>,
    section_index: &mut usize,
    chapter_line: &mut Option<usize>,
    config: &AozoraConfig,
    bare: bool,
) {
    if matches!(
        page_marker,
        Some(PAGE_CHAPTER_MIDDLE_MARKER | PAGE_CHAPTER_BOTTOM_MARKER)
    ) && current.is_empty()
        && line.trim().is_empty()
    {
        return;
    }
    if is_colophon_line(line) && !current.is_empty() {
        trim_trailing_empty_lines(current);
        no_br.truncate(current.len());
        if !current.is_empty() {
            push_rendered_section(sections, current, no_br, chapter_line, *page_marker, config);
            *section_index += 1;
        }
        *page_marker = Some(PAGE_NO_CHAPTER_MARKER);
    }
    if should_force_page_break(current, line, config) {
        trim_trailing_empty_lines(current);
        no_br.truncate(current.len());
        if !current.is_empty() {
            push_rendered_section(sections, current, no_br, chapter_line, *page_marker, config);
            *section_index += 1;
        }
        *page_marker = None;
    }
    current.push(line.to_owned());
    no_br.push(bare);
}

/// Normalizes a chapter label the way the reference pre-read does:
/// suffix notes keep their target text, ruby readings and note markers are
/// removed, symbol runs collapse, and the label is truncated at 64 chars.
fn chapter_name(line: &str, config: &AozoraConfig) -> String {
    let mut name = line.to_owned();
    // Suffix notes (［＃「X」…］) keep their target text.
    while let Some(start) = name.find("［＃「") {
        let note_start = start + "［＃「".len();
        let Some(quote_end) = name[note_start..].find('」') else {
            break;
        };
        let target_end = note_start + quote_end;
        let target = name[note_start..target_end].to_owned();
        let Some(close) = name[target_end + '」'.len_utf8()..].find('］') else {
            break;
        };
        let suffix_end = target_end + '」'.len_utf8() + close + '］'.len_utf8();
        let suffix = &name[target_end + '」'.len_utf8()..suffix_end - '］'.len_utf8()];
        if suffix_rule_known(suffix, config) {
            // The target usually already precedes the note (［＃…］text…):
            // drop the note then, otherwise substitute the target text.
            let before = &name[..start];
            if before.ends_with(&target) || strip_markers(before).ends_with(&target) {
                name.replace_range(start..suffix_end, "");
            } else {
                name.replace_range(start..suffix_end, &target);
            }
        } else {
            break;
        }
    }
    // Remove ruby readings: strip 《…》 and the leading ｜. ※-escaped
    // brackets (※《※》…※》※) are literal text and are kept.
    let mut stripped = String::with_capacity(name.len());
    let mut in_ruby = false;
    let mut escaped_count = 0usize;
    for character in name.chars() {
        let escaped = if character == '※' {
            escaped_count += 1;
            escaped_count % 2 == 1
        } else {
            escaped_count = 0;
            false
        };
        if in_ruby {
            if character == '》' && !escaped {
                in_ruby = false;
            }
            if escaped {
                stripped.push(character);
            }
        } else {
            match character {
                '｜' if !escaped => {}
                '《' if !escaped => in_ruby = true,
                _ => stripped.push(character),
            }
        }
    }
    let name = stripped;
    // Remove remaining note markers and ※-escapes, then trim.
    // 外字注記（米印→※、二重山括弧→《》等）は変換し、※プレフィクスも消費する。
    let mut cleaned = String::with_capacity(name.len());
    let mut rest = name.as_str();
    while let Some(start) = rest.find("［＃") {
        cleaned.push_str(&rest[..start]);
        let after = &rest[start + "［＃".len()..];
        let Some(close) = after.find('］') else {
            cleaned.push_str(rest);
            rest = "";
            break;
        };
        let note = &after[..close];
        let gaiji_key = note.split(['、', ',']).next().unwrap_or(note);
        let replacement = config
            .gaiji
            .get(note)
            .or_else(|| config.gaiji.get(&format!("［＃{note}］")))
            .or_else(|| config.gaiji.get(gaiji_key))
            .or_else(|| config.gaiji.get(&format!("［＃{gaiji_key}］")))
            .cloned()
            .or_else(|| {
                // 画像注記（（…。…））は本文から消える（Java: 注記→img→除去）
                let has_image_syntax =
                    note.contains('（') && note.contains('）') && note.contains('.');
                // コード付き外字注記も同様
                let has_code = note.contains('、')
                    && (note.contains('-') || note.contains("U+") || note.contains("u+"));
                (has_image_syntax || has_code).then(String::new)
            });
        if let Some(replacement) = replacement {
            // ※プレフィクスは注記と一体で変換される
            if cleaned.ends_with('※') {
                cleaned.pop();
            }
            cleaned.push_str(&replacement);
        }
        rest = &after[close + '］'.len_utf8()..];
    }
    cleaned.push_str(rest);
    let name = cleaned;
    let mut name = name.replace('\t', " ");
    name = name
        .trim_start_matches([' ', '\u{3000}'])
        .trim_end_matches([' ', '\u{3000}'])
        .to_owned();
    // Collapse runs of separators to a single character.
    let mut reduced = String::with_capacity(name.len());
    let mut previous_separator = None;
    for character in name.chars() {
        let separator = matches!(character, '=' | '＝' | '-' | '―' | '─');
        if separator && previous_separator == Some(character) {
            continue;
        }
        reduced.push(character);
        previous_separator = if separator { Some(character) } else { None };
    }
    let name = reduced;
    // Remove img/a tags the way the reference pre-read does.
    let mut without_tags = String::with_capacity(name.len());
    let mut rest = name.as_str();
    while let Some(start) = rest.find('<') {
        without_tags.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('>') else {
            without_tags.push_str(rest);
            rest = "";
            break;
        };
        let tag = &rest[start + 1..start + end];
        let tag_name = tag
            .trim_start_matches(['/', ' '])
            .split(|c: char| c.is_ascii_whitespace())
            .next()
            .unwrap_or("");
        if tag_name.eq_ignore_ascii_case("img") || tag_name.eq_ignore_ascii_case("a") {
            rest = &rest[start + end + 1..];
        } else {
            without_tags.push_str(&rest[start..start + end + 1]);
            rest = &rest[start + end + 1..];
        }
    }
    without_tags.push_str(rest);
    let mut name = without_tags;
    if name.chars().count() > 64 {
        name = name.chars().take(64).collect::<String>() + "...";
    }
    name
}

/// Strips note markers and ruby readings so a suffix-note target can be
/// matched against the text that precedes it.
fn strip_markers(input: &str) -> String {
    let mut stripped = String::with_capacity(input.len());
    let mut in_ruby = false;
    let mut rest = input;
    while let Some(start) = rest.find("［＃") {
        stripped.push_str(&rest[..start]);
        let after = &rest[start + "［＃".len()..];
        let Some(close) = after.find('］') else {
            stripped.push_str(rest);
            return stripped;
        };
        rest = &after[close + '］'.len_utf8()..];
    }
    stripped.push_str(rest);
    let mut without_ruby = String::with_capacity(stripped.len());
    for character in stripped.chars() {
        if in_ruby {
            if character == '》' {
                in_ruby = false;
            }
        } else {
            match character {
                '｜' => {}
                '《' => in_ruby = true,
                _ => without_ruby.push(character),
            }
        }
    }
    without_ruby
}

fn suffix_rule_known(suffix: &str, config: &AozoraConfig) -> bool {
    config
        .suffix_notes
        .keys()
        .any(|key| key == suffix || key.ends_with(suffix))
}

/// True when the name consists only of decorative symbols and spaces.
fn symbols_only(name: &str) -> bool {
    name.chars().all(|character| {
        matches!(
            character,
            '◇' | '◆' | '□' | '■' | '▽' | '▼' | '☆' | '★' | '＊' | '＋' | '×' | '†' | '\u{3000}'
        )
    })
}

fn should_force_page_break(current: &[String], line: &str, config: &AozoraConfig) -> bool {
    if !config.force_page_break || current.is_empty() || line.trim().is_empty() {
        return false;
    }
    let page_size = current
        .iter()
        .map(|value| value.len().saturating_add(8))
        .sum::<usize>();
    if config.force_page_break_size > 0 && page_size > config.force_page_break_size {
        return true;
    }
    let empty_lines = current
        .iter()
        .rev()
        .take_while(|value| value.trim().is_empty())
        .count();
    if config.force_page_break_empty_line > 0
        && empty_lines >= config.force_page_break_empty_line
        && page_size > config.force_page_break_empty_size
    {
        return true;
    }
    config.force_page_break_chapter_level > 0
        && page_size > config.force_page_break_chapter_size
        && is_chapter_line(line)
}

fn is_colophon_line(line: &str) -> bool {
    line.trim_start_matches([' ', '\u{3000}'])
        .starts_with("底本：")
}

fn is_chapter_line(line: &str) -> bool {
    let Some((note, _)) = heading_note_at_start(line) else {
        return false;
    };
    heading_spec(note).is_some() || note.contains("見出し")
}

fn trim_trailing_empty_lines(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
}

fn find_page_break_note(line: &str, config: &AozoraConfig) -> Option<(usize, usize, String)> {
    config
        .page_break_notes
        .iter()
        .filter_map(|note| {
            let marker = format!("［＃{note}］");
            line.find(&marker)
                .map(|offset| (offset, offset + marker.len(), note.clone()))
        })
        .min_by_key(|(offset, _, _)| *offset)
}

const PAGE_NO_CHAPTER_MARKER: &str = "<!-- aozora-page-no-chapter -->";
const PAGE_CHAPTER_MARKER: &str = "<!-- aozora-page-chapter -->";
const PAGE_CHAPTER_MIDDLE_MARKER: &str = "<!-- aozora-page-middle --><!-- aozora-page-chapter -->";
const PAGE_CHAPTER_BOTTOM_MARKER: &str = "<!-- aozora-page-bottom --><!-- aozora-page-chapter -->";
const RAW_COMMENT_PREFIX: &str = "\u{0000}aozora-raw-comment\u{0000}";

fn render_marked_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    no_br: &[bool],
    chapter_line: Option<usize>,
    config: &AozoraConfig,
    marker: Option<&str>,
) -> String {
    let fragment = render_lines(lines, no_br, chapter_line, config);
    marker
        .map(|marker| format!("{marker}\n{fragment}"))
        .unwrap_or(fragment)
}

#[derive(Clone, Copy)]
struct HeadingSpec {
    element: &'static str,
    class_name: &'static str,
}

enum OpenBlock {
    Generated { close_tag: String },
    Configured { fallback_close_tag: String },
}

fn render_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    no_br: &[bool],
    chapter_line: Option<usize>,
    config: &AozoraConfig,
) -> String {
    let mut fragment = String::new();
    let mut has_line = false;
    let mut blocks: Vec<OpenBlock> = Vec::new();
    let mut pending_heading: Option<HeadingSpec> = None;
    let mut pending_config_heading: Option<(String, String)> = None;
    let mut output_count = 0usize;
    let mut chapter_done = false;

    let block_markers = config
        .block_open_tags
        .keys()
        .chain(config.block_close_tags.keys())
        .map(|note| format!("［＃{note}］"))
        .collect::<Vec<_>>();
    let expanded_lines = lines
        .into_iter()
        .enumerate()
        .flat_map(|(index, line)| {
            split_block_notes(line, &block_markers)
                .into_iter()
                .map(move |piece| (index, piece))
        })
        .collect::<Vec<_>>();

    for (line_index, line) in expanded_lines.iter().map(|(index, line)| (*index, line.as_str())) {
        has_line = true;
        let line_no_br = no_br.get(line_index).copied().unwrap_or(false);
        let chapter_id = if !chapter_done && chapter_line == Some(line_index) {
            chapter_done = true;
            Some(format!("kobo.{}.1", output_count + 1))
        } else {
            None
        };
        if line_no_br {
            // 前-part of a mid-line page break: output bare, no <p> wrapper.
            output_count += 1;
            let converted = convert_inline(line, config);
            let converted = chapter_id
                .map(|id| inject_kobo_id(&converted, &id))
                .unwrap_or(converted);
            fragment.push_str(&converted);
            fragment.push('\n');
            continue;
        }
        if let Some(raw) = line.strip_prefix(RAW_COMMENT_PREFIX) {
            output_count += 1;
            fragment.push_str("<p>");
            fragment.push_str(raw);
            fragment.push_str("</p>\n");
            continue;
        }
        if let Some((start, end, open_tag, close_tag, no_newline)) =
            find_inline_block_note(line, config)
        {
            output_count += 1;
            fragment.push_str(&convert_inline(&line[..start], config));
            let open_tag = chapter_id
                .map(|id| inject_kobo_id(&open_tag, &id))
                .unwrap_or(open_tag);
            fragment.push_str(&open_tag);
            fragment.push_str(&convert_inline(&line[end..], config));
            fragment.push_str(&close_tag);
            if !no_newline {
                fragment.push('\n');
            }
            continue;
        }
        let trimmed = line.trim();

        if let Some((open_tag, close_tag)) = pending_config_heading.take() {
            output_count += 1;
            let open_tag = chapter_id
                .map(|id| inject_kobo_id(&open_tag, &id))
                .unwrap_or(open_tag);
            fragment.push_str(&open_tag);
            fragment.push_str(&convert_inline(line, config));
            fragment.push_str(&close_tag);
            fragment.push('\n');
            continue;
        }

        if let Some(spec) = pending_heading.take() {
            append_heading(
                &mut fragment,
                spec,
                line,
                config,
                &mut output_count,
                chapter_id.as_deref(),
            );
            continue;
        }

        if !blocks.is_empty() {
            if let Some((note, rest)) = heading_note_at_start(line)
                && !rest.trim().is_empty()
                && let Some((open_tag, close_tag)) = config.block_inline_tags.get(note)
            {
                output_count += 1;
                let open_tag = chapter_id
                    .map(|id| inject_kobo_id(open_tag, &id))
                    .unwrap_or_else(|| open_tag.clone());
                fragment.push_str(&open_tag);
                fragment.push_str(&convert_inline(rest.trim_start(), config));
                fragment.push_str(close_tag);
                fragment.push('\n');
                continue;
            }

            if let Some((note, rest)) = heading_note_at_start(line)
                && rest.trim().is_empty()
            {
                let closes_generated = matches!(blocks.last(), Some(OpenBlock::Generated { .. }))
                    && is_indent_close_note(note);
                if closes_generated {
                    if let Some(OpenBlock::Generated { close_tag }) = blocks.pop() {
                        output_count += 1;
                        fragment.push_str(&close_tag);
                        fragment.push('\n');
                    }
                    continue;
                }

                let closes_configured = matches!(blocks.last(), Some(OpenBlock::Configured { .. }));
                if closes_configured && let Some(close_tag) = config.block_close_tags.get(note) {
                    output_count += 1;
                    fragment.push_str(close_tag);
                    fragment.push('\n');
                    if close_tag == "</span>" && image_wrapper_is_open(&fragment) {
                        fragment.push_str("</span>");
                        fragment.push('\n');
                    }
                    blocks.pop();
                    continue;
                }
                if let Some((open_tag, close_tag)) = generated_indent_block(note) {
                    output_count += 1;
                    fragment.push_str(&open_tag);
                    fragment.push('\n');
                    blocks.push(OpenBlock::Generated { close_tag });
                    continue;
                }

                if let Some(open_tag) = config.block_open_tags.get(note) {
                    output_count += 1;
                    fragment.push_str(open_tag);
                    fragment.push('\n');
                    blocks.push(OpenBlock::Configured {
                        fallback_close_tag: fallback_close_tag(open_tag),
                    });
                    continue;
                }
                if let Some(tag) = config.block_single_tags.get(note) {
                    output_count += 1;
                    fragment.push_str(tag);
                    fragment.push('\n');
                    continue;
                }
                if let Some((open_tag, close_tag)) = config.block_inline_tags.get(note) {
                    pending_config_heading = Some((open_tag.clone(), close_tag.clone()));
                    continue;
                }
            }
            if !blocks.is_empty() {
                append_block_line(
                    &mut fragment,
                    line,
                    config,
                    &mut output_count,
                    chapter_id.as_deref(),
                );
                continue;
            }
        }

        if let Some(note) = page_break_note(trimmed)
            && let Some(close_tag) = config.block_close_tags.get(note)
        {
            output_count += 1;
            fragment.push_str(close_tag);
            fragment.push('\n');
            continue;
        }

        if let Some((note, rest)) = heading_note_at_start(line) {
            if let Some(spec) = heading_spec(note) {
                let content = heading_content(note, rest);
                if content.trim().is_empty() {
                    pending_heading = Some(spec);
                } else {
                    // Java: 行頭の全角/半角空白は見出しタグの前に出力される
                    let leading_len = line.len() - line.trim_start().len();
                    fragment.push_str(&line[..leading_len]);
                    append_heading(
                        &mut fragment,
                        spec,
                        content,
                        config,
                        &mut output_count,
                        chapter_id.as_deref(),
                    );
                }
                continue;
            }
            if let Some((open_tag, close_tag)) = generated_indent_block(note) {
                output_count += 1;
                let open_tag = chapter_id
                    .map(|id| inject_kobo_id(&open_tag, &id))
                    .unwrap_or(open_tag);
                fragment.push_str(&open_tag);
                if !rest.trim().is_empty() {
                    fragment.push_str(&convert_inline(rest.trim_start(), config));
                    fragment.push('\n');
                } else {
                    fragment.push('\n');
                }
                blocks.push(OpenBlock::Generated { close_tag });
                continue;
            }
            if let Some(tag) = config.block_single_tags.get(note) {
                output_count += 1;
                let tag = chapter_id
                    .map(|id| inject_kobo_id(tag, &id))
                    .unwrap_or_else(|| tag.to_owned());
                fragment.push_str(&tag);
                if !rest.trim().is_empty() {
                    fragment.push_str(&convert_inline(rest.trim_start(), config));
                }
                fragment.push('\n');
                continue;
            }
            if let Some((open_tag, close_tag)) = config.block_inline_tags.get(note) {
                if rest.trim().is_empty() {
                    pending_config_heading = Some((open_tag.clone(), close_tag.clone()));
                } else {
                    output_count += 1;
                    let open_tag = chapter_id
                        .map(|id| inject_kobo_id(open_tag, &id))
                        .unwrap_or_else(|| open_tag.clone());
                    fragment.push_str(&open_tag);
                    fragment.push_str(&convert_inline(rest.trim_start(), config));
                    fragment.push_str(close_tag);
                    fragment.push('\n');
                }
                continue;
            }
            if let Some(open_tag) = config.block_open_tags.get(note) {
                output_count += 1;
                let open_tag = chapter_id
                    .map(|id| inject_kobo_id(open_tag, &id))
                    .unwrap_or_else(|| open_tag.to_owned());
                fragment.push_str(&open_tag);
                if !rest.trim().is_empty() {
                    fragment.push_str(&convert_inline(rest.trim_start(), config));
                    fragment.push('\n');
                } else {
                    fragment.push('\n');
                }
                blocks.push(OpenBlock::Configured {
                    fallback_close_tag: fallback_close_tag(&open_tag),
                });
                continue;
            }
        }

        append_line(&mut fragment, line, config, &mut output_count, chapter_id.as_deref());
    }

    while let Some(block) = blocks.pop() {
        output_count += 1;
        match block {
            OpenBlock::Generated { close_tag } => {
                fragment.push_str(&close_tag);
                fragment.push('\n');
            }
            OpenBlock::Configured { fallback_close_tag } => {
                fragment.push_str(&fallback_close_tag);
                fragment.push('\n');
            }
        }
    }
    if let Some((open_tag, close_tag)) = pending_config_heading {
        fragment.push_str(&open_tag);
        fragment.push_str(&close_tag);
        fragment.push('\n');
    } else if let Some(spec) = pending_heading {
        append_heading(&mut fragment, spec, "", config, &mut output_count, None);
    }

    if !has_line {
        fragment.push_str("    <p><br/></p>\n");
    }
    if config.ini.get_bool("MarkId").unwrap_or(false) {
        fragment = add_kobo_ids(&fragment);
    }
    balance_xhtml(&fragment)
}

fn add_kobo_ids(fragment: &str) -> String {
    let mut output = String::with_capacity(fragment.len());
    let mut cursor = 0;
    let mut line_id = 0usize;
    while let Some(relative_start) = fragment[cursor..].find("<p") {
        let start = cursor + relative_start;
        let Some(relative_end) = fragment[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &fragment[start..end];
        let is_paragraph = tag
            .as_bytes()
            .get(2)
            .is_some_and(|character| character.is_ascii_whitespace() || *character == b'>');
        output.push_str(&fragment[cursor..start]);
        if is_paragraph {
            line_id += 1;
            if tag.contains(" id=") {
                output.push_str(tag);
            } else {
                output.push_str(tag.strip_suffix('>').unwrap_or(tag));
                output.push_str(&format!(" id=\"kobo.{line_id}.1\">"));
            }
        } else {
            output.push_str(tag);
        }
        cursor = end;
    }
    output.push_str(&fragment[cursor..]);
    output
}

fn balance_xhtml(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut open_tags: Vec<String> = Vec::new();
    let mut cursor = 0;
    while let Some(relative_start) = input[cursor..].find('<') {
        let start = cursor + relative_start;
        output.push_str(&input[cursor..start]);
        let Some(relative_end) = input[start..].find('>') else {
            output.push_str(&input[start..]);
            break;
        };
        let end = start + relative_end + 1;
        let tag = &input[start..end];
        if tag.starts_with("<!--") {
            output.push_str(tag);
        } else if let Some(name) = tag
            .strip_prefix("</")
            .and_then(|value| value.strip_suffix('>'))
            .map(str::trim)
            .filter(|name| !name.is_empty())
        {
            if let Some(position) = open_tags.iter().rposition(|open| open == name) {
                while open_tags.len() > position + 1 {
                    if let Some(open) = open_tags.pop() {
                        output.push_str("</");
                        output.push_str(&open);
                        output.push('>');
                    }
                }
                open_tags.pop();
                output.push_str(tag);
            }
        } else if let Some(name) = tag
            .strip_prefix('<')
            .and_then(|value| value.strip_suffix('>'))
            .map(str::trim)
            .and_then(|value| value.split_whitespace().next())
            .filter(|name| !name.starts_with('/') && !name.starts_with('!'))
        {
            output.push_str(tag);
            let self_closing = tag.trim_end().ends_with("/>")
                || matches!(
                    name,
                    "area"
                        | "base"
                        | "br"
                        | "col"
                        | "embed"
                        | "hr"
                        | "img"
                        | "input"
                        | "link"
                        | "meta"
                        | "param"
                        | "source"
                        | "track"
                        | "wbr"
                );
            if !self_closing {
                open_tags.push(name.to_owned());
            }
        } else {
            output.push_str(tag);
        }
        cursor = end;
    }
    if cursor < input.len() {
        output.push_str(&input[cursor..]);
    }
    while let Some(open) = open_tags.pop() {
        output.push_str("</");
        output.push_str(&open);
        output.push('>');
    }
    output
}

fn split_block_notes(line: &str, markers: &[String]) -> Vec<String> {
    if line.starts_with(RAW_COMMENT_PREFIX) {
        return vec![line.to_owned()];
    }
    let mut pieces = Vec::new();
    let mut rest = line;
    while let Some((offset, marker)) = markers
        .iter()
        .filter_map(|marker| rest.find(marker).map(|offset| (offset, marker)))
        .min_by_key(|(offset, _)| *offset)
    {
        if offset > 0 {
            pieces.push(rest[..offset].to_owned());
        }
        pieces.push(marker.clone());
        rest = &rest[offset + marker.len()..];
    }
    if !rest.is_empty() || pieces.is_empty() {
        pieces.push(rest.to_owned());
    }
    pieces
}
fn find_inline_block_note(
    line: &str,
    config: &AozoraConfig,
) -> Option<(usize, usize, String, String, bool)> {
    for (start, _) in line.match_indices("［＃") {
        let note_start = start + "［＃".len();
        let close = note_start + line[note_start..].find('］')?;
        let end = close + '］'.len_utf8();
        let note = &line[note_start..close];
        let Some((open_tag, close_tag)) = config.block_inline_tags.get(note) else {
            continue;
        };
        if start == 0 || line[..start].trim().is_empty() {
            continue;
        }
        let closing_note = format!("［＃{note}終わり］");
        if line[end..].contains(&closing_note) {
            continue;
        }
        return Some((
            start,
            end,
            open_tag.clone(),
            close_tag.clone(),
            config.block_open_tags.contains_key(note),
        ));
    }
    None
}

fn heading_note_at_start(line: &str) -> Option<(&str, &str)> {
    let line = line.trim_start();
    let rest = line.strip_prefix("［＃")?;
    let close = rest.find('］')?;
    let note = &rest[..close];
    let content = &rest[close + '］'.len_utf8()..];
    Some((note, content))
}

fn heading_content<'a>(note: &str, content: &'a str) -> &'a str {
    let Some(close_note) = (match note {
        "見出し" => Some("見出し終わり"),
        "大見出し" => Some("大見出し終わり"),
        "中見出し" => Some("中見出し終わり"),
        "小見出し" => Some("小見出し終わり"),
        _ => None,
    }) else {
        return content;
    };
    let marker = format!("［＃{close_note}］");
    content
        .strip_suffix(&marker)
        .map(str::trim_end)
        .unwrap_or(content)
}

fn page_break_note(line: &str) -> Option<&str> {
    line.trim().strip_prefix("［＃")?.strip_suffix('］')
}

fn heading_spec(note: &str) -> Option<HeadingSpec> {
    match note {
        "見出し" => Some(HeadingSpec {
            element: "h1",
            class_name: "font-1em50",
        }),
        "大見出し" => Some(HeadingSpec {
            element: "h1",
            class_name: "font-1em50",
        }),
        "中見出し" => Some(HeadingSpec {
            element: "h2",
            class_name: "font-1em30",
        }),
        "小見出し" => Some(HeadingSpec {
            element: "h3",
            class_name: "font-1em10",
        }),
        _ => None,
    }
}

fn generated_indent_block(note: &str) -> Option<(String, String)> {
    let rest = note.strip_prefix("ここから")?;
    let (indent, rest) = parse_fullwidth_number(rest)?;
    let rest = rest.strip_prefix("字下げ")?;
    let rest = rest.strip_prefix('、')?;

    let (class_name, _) = if let Some(rest) = rest.strip_prefix("折り返して") {
        let (wrapped, rest) = parse_fullwidth_number(rest)?;
        let rest = rest.strip_prefix("字下げ")?;
        (
            format!("pt{wrapped} idt{}", indent as isize - wrapped as isize),
            rest,
        )
    } else if let Some(width) = parse_fullwidth_number(rest)
        .and_then(|(width, rest)| rest.strip_prefix("字詰め").map(|_| width))
    {
        (format!("pt{indent} jzm{width}"), "")
    } else {
        let mut classes = vec![format!("mt{indent}")];
        for (needle, class) in [
            ("破線罫囲み", "dashed_border"),
            ("罫囲み", "border"),
            ("破線枠囲み", "dashed_border"),
            ("枠囲み", "border"),
            ("中央揃え", "center"),
            ("横書き", "yoko"),
        ] {
            if rest.contains(needle) {
                classes.push(class.to_owned());
            }
        }
        (classes.join(" "), rest)
    };
    Some((format!("<div class=\"{class_name}\">"), "</div>".to_owned()))
}

fn parse_fullwidth_number(input: &str) -> Option<(usize, &str)> {
    let mut value = 0usize;
    let mut end = 0;
    for (index, character) in input.char_indices() {
        let digit = match character {
            '０'..='９' => character as u32 - '０' as u32,
            '0'..='9' => character as u32 - '0' as u32,
            _ => break,
        };
        value = value.checked_mul(10)?.checked_add(digit as usize)?;
        end = index + character.len_utf8();
    }
    (end > 0).then_some((value, &input[end..]))
}

fn is_indent_close_note(note: &str) -> bool {
    note.strip_prefix("ここで字下げ")
        .is_some_and(|rest| rest == "終わり" || rest == "終り" || rest.ends_with("終わり"))
}

fn fallback_close_tag(open_tag: &str) -> String {
    let tag_name = open_tag
        .strip_prefix('<')
        .and_then(|value| value.split([' ', '>']).next())
        .filter(|value| !value.is_empty())
        .unwrap_or("div");
    format!("</{tag_name}>")
}

fn append_heading(
    fragment: &mut String,
    spec: HeadingSpec,
    text: &str,
    config: &AozoraConfig,
    output_count: &mut usize,
    chapter_id: Option<&str>,
) {
    *output_count += 1;
    fragment.push('<');
    fragment.push_str(spec.element);
    if let Some(id) = chapter_id {
        fragment.push_str(" id=\"");
        fragment.push_str(id);
        fragment.push('"');
    }
    fragment.push_str(" class=\"");
    fragment.push_str(spec.class_name);
    fragment.push_str("\">");
    fragment.push_str(&convert_inline(text, config));
    fragment.push_str("</");
    fragment.push_str(spec.element);
    fragment.push_str(">\n");
}

/// Injects a kobo id the way the reference renderer does: into the first tag
/// of a block line, or wrapping the first character of a bare line.
fn inject_kobo_id(line: &str, id: &str) -> String {
    let bytes = line.as_bytes();
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(0);
    if bytes.get(start) == Some(&b'<') {
        let mut tag_end = start + 1;
        while tag_end < bytes.len()
            && (bytes[tag_end].is_ascii_alphanumeric() || bytes[tag_end] == b'|')
        {
            tag_end += 1;
        }
        format!("{} id=\"{id}\"{}", &line[..tag_end], &line[tag_end..])
    } else {
        let rest = &line[start..];
        let first = rest.chars().next().unwrap_or(' ');
        let after = &rest[first.len_utf8()..];
        format!("{}<span id=\"{id}\">{first}</span>{after}", &line[..start])
    }
}

fn image_wrapper_is_open(value: &str) -> bool {
    let mut search_end = value.len();
    while let Some(start) = value[..search_end].rfind("<span") {
        let tail = &value[start..];
        if tail.contains("<img") && tail.matches("<span").count() > tail.matches("</span>").count()
        {
            return true;
        }
        search_end = start;
    }
    false
}

fn append_open_image_line(fragment: &mut String, converted: &str) -> bool {
    let already_open = image_wrapper_is_open(fragment);
    if !already_open && !image_wrapper_is_open(converted) {
        return false;
    }
    fragment.push_str(converted);
    if already_open && converted.contains("class=\"caption") {
        fragment.push('\n');
        fragment.push_str("</span>");
    }
    fragment.push('\n');
    true
}

fn append_line(
    fragment: &mut String,
    line: &str,
    config: &AozoraConfig,
    output_count: &mut usize,
    chapter_id: Option<&str>,
) {
    let converted = convert_inline(line, config);
    if append_open_image_line(fragment, &converted) {
        return;
    }
    if converted.trim().is_empty() {
        fragment.push_str("    <p><br/></p>\n");
    } else {
        *output_count += 1;
        // The reference renderer does not attach kobo ids to <p>-wrapped
        // page-break chapters, so `chapter_id` is intentionally unused here.
        let _ = chapter_id;
        // 見出し注記で生成された h1/h2/h3 は <p> で包まない（Java 準拠）
        let is_heading = ["<h1", "<h2", "<h3"]
            .iter()
            .any(|tag| converted.contains(tag));
        if is_heading {
            fragment.push_str(&converted);
            fragment.push('\n');
        } else {
            fragment.push_str("    <p>");
            fragment.push_str(&converted);
            fragment.push_str("</p>\n");
        }
    }
}
fn append_block_line(
    fragment: &mut String,
    line: &str,
    config: &AozoraConfig,
    output_count: &mut usize,
    chapter_id: Option<&str>,
) {
    let converted = convert_inline(line, config);
    if append_open_image_line(fragment, &converted) {
        return;
    }
    if converted.trim().is_empty() {
        fragment.push_str("<p><br/></p>\n");
    } else {
        *output_count += 1;
        let _ = chapter_id;
        fragment.push_str("<p>");
        fragment.push_str(&converted);
        fragment.push_str("</p>\n");
    }
}
#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
