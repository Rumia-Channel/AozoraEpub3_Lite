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
    Ok(render_lines(lines.iter().map(String::as_str), config))
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

pub fn aozora_text_to_xhtml_sections(input: &str) -> Result<Vec<String>, TextError> {
    aozora_text_to_xhtml_sections_with_config(input, &AozoraConfig::default())
}

pub fn aozora_text_to_xhtml_sections_with_config(
    input: &str,
    config: &AozoraConfig,
) -> Result<Vec<String>, TextError> {
    let mut sections = Vec::new();
    let mut current = Vec::new();
    let mut page_marker = None;

    for line in visible_lines(input, config).iter() {
        let mut remainder = line.as_str();
        loop {
            let Some((offset, end, note)) = find_page_break_note(remainder, config) else {
                append_section_line(
                    remainder,
                    &mut sections,
                    &mut current,
                    &mut page_marker,
                    config,
                );
                break;
            };

            if !remainder[..offset].is_empty() {
                append_section_line(
                    &remainder[..offset],
                    &mut sections,
                    &mut current,
                    &mut page_marker,
                    config,
                );
            }
            if config.split_page_breaks {
                trim_trailing_empty_lines(&mut current);
                if !current.is_empty() {
                    sections.push(render_marked_lines(
                        current.iter().map(String::as_str),
                        config,
                        page_marker,
                    ));
                    current.clear();
                }
                page_marker = if config.page_middle_notes.contains(&note) {
                    Some(PAGE_MIDDLE_MARKER)
                } else if config.page_bottom_notes.contains(&note) {
                    Some(PAGE_BOTTOM_MARKER)
                } else {
                    None
                };
            }
            remainder = &remainder[end..];
            if remainder.is_empty() {
                break;
            }
        }
    }

    trim_trailing_empty_lines(&mut current);
    if !current.is_empty() || sections.is_empty() {
        sections.push(render_marked_lines(
            current.iter().map(String::as_str),
            config,
            page_marker,
        ));
    }

    Ok(sections)
}

fn append_section_line(
    line: &str,
    sections: &mut Vec<String>,
    current: &mut Vec<String>,
    page_marker: &mut Option<&'static str>,
    config: &AozoraConfig,
) {
    if line.trim().is_empty() && current.is_empty() && page_marker.is_some() {
        return;
    }
    if is_colophon_line(line) && !current.is_empty() {
        trim_trailing_empty_lines(current);
        if !current.is_empty() {
            sections.push(render_marked_lines(
                current.iter().map(String::as_str),
                config,
                *page_marker,
            ));
            current.clear();
        }
        *page_marker = Some(PAGE_NO_CHAPTER_MARKER);
    }
    if should_force_page_break(current, line, config) {
        trim_trailing_empty_lines(current);
        if !current.is_empty() {
            sections.push(render_marked_lines(
                current.iter().map(String::as_str),
                config,
                *page_marker,
            ));
            current.clear();
        }
        *page_marker = None;
    }
    current.push(line.to_owned());
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
    heading_spec(note).is_some() || block_heading_spec(note).is_some() || note.contains("見出し")
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

const PAGE_MIDDLE_MARKER: &str = "<!-- aozora-page-middle -->";
const PAGE_BOTTOM_MARKER: &str = "<!-- aozora-page-bottom -->";
const PAGE_NO_CHAPTER_MARKER: &str = "<!-- aozora-page-no-chapter -->";
const RAW_COMMENT_PREFIX: &str = "\u{0000}aozora-raw-comment\u{0000}";

fn render_marked_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    config: &AozoraConfig,
    marker: Option<&str>,
) -> String {
    let fragment = render_lines(lines, config);
    marker
        .map(|marker| format!("{marker}\n{fragment}"))
        .unwrap_or(fragment)
}

#[derive(Clone, Copy)]
struct HeadingSpec {
    element: &'static str,
    class_name: &'static str,
    close_note: &'static str,
}

enum OpenBlock {
    Hardcoded(HeadingSpec),
    Generated { close_tag: String },
    Configured { fallback_close_tag: String },
}

fn render_lines<'a>(lines: impl IntoIterator<Item = &'a str>, config: &AozoraConfig) -> String {
    let mut fragment = String::new();
    let mut has_line = false;
    let mut blocks: Vec<OpenBlock> = Vec::new();
    let mut pending_heading: Option<HeadingSpec> = None;
    let mut pending_config_heading: Option<(String, String)> = None;

    let block_markers = config
        .block_open_tags
        .keys()
        .chain(config.block_close_tags.keys())
        .map(|note| format!("［＃{note}］"))
        .collect::<Vec<_>>();
    let expanded_lines = lines
        .into_iter()
        .flat_map(|line| split_block_notes(line, &block_markers))
        .collect::<Vec<_>>();

    for line in expanded_lines.iter().map(String::as_str) {
        has_line = true;
        if let Some(raw) = line.strip_prefix(RAW_COMMENT_PREFIX) {
            fragment.push_str("<p>");
            fragment.push_str(raw);
            fragment.push_str("</p>\n");
            continue;
        }
        let trimmed = line.trim();

        if let Some((open_tag, close_tag)) = pending_config_heading.take() {
            fragment.push_str(&open_tag);
            fragment.push_str(&convert_inline(line, config));
            fragment.push_str(&close_tag);
            fragment.push('\n');
            continue;
        }

        if let Some(spec) = pending_heading.take() {
            append_heading(&mut fragment, spec, line, config);
            continue;
        }

        if !blocks.is_empty() {
            if let Some((note, rest)) = heading_note_at_start(line)
                && !rest.trim().is_empty()
                && let Some((open_tag, close_tag)) = config.block_inline_tags.get(note)
            {
                fragment.push_str(open_tag);
                fragment.push_str(&convert_inline(rest.trim_start(), config));
                fragment.push_str(close_tag);
                fragment.push('\n');
                continue;
            }

            if let Some((note, rest)) = heading_note_at_start(line)
                && rest.trim().is_empty()
            {
                let closes_hardcoded = matches!(blocks.last(), Some(OpenBlock::Hardcoded(spec)) if note == spec.close_note);
                if closes_hardcoded {
                    if let Some(OpenBlock::Hardcoded(spec)) = blocks.pop() {
                        fragment.push_str("</");
                        fragment.push_str(spec.element);
                        fragment.push_str(">\n");
                    }
                    continue;
                }

                let closes_generated = matches!(blocks.last(), Some(OpenBlock::Generated { .. }))
                    && is_indent_close_note(note);
                if closes_generated {
                    if let Some(OpenBlock::Generated { close_tag }) = blocks.pop() {
                        fragment.push_str(&close_tag);
                        fragment.push('\n');
                    }
                    continue;
                }

                let closes_configured = matches!(blocks.last(), Some(OpenBlock::Configured { .. }));
                if closes_configured && let Some(close_tag) = config.block_close_tags.get(note) {
                    fragment.push_str(close_tag);
                    fragment.push('\n');
                    blocks.pop();
                    continue;
                }
                if let Some((open_tag, close_tag)) = generated_indent_block(note) {
                    fragment.push_str(&open_tag);
                    blocks.push(OpenBlock::Generated { close_tag });
                    continue;
                }

                if let Some(spec) = block_heading_spec(note) {
                    fragment.push('<');
                    fragment.push_str(spec.element);
                    fragment.push_str(" class=\"");
                    fragment.push_str(spec.class_name);
                    fragment.push_str("\">");
                    blocks.push(OpenBlock::Hardcoded(spec));
                    continue;
                }
                if let Some(open_tag) = config.block_open_tags.get(note) {
                    fragment.push_str(open_tag);
                    blocks.push(OpenBlock::Configured {
                        fallback_close_tag: fallback_close_tag(open_tag),
                    });
                    continue;
                }
                if let Some(tag) = config.block_single_tags.get(note) {
                    fragment.push_str(tag);
                    fragment.push('\n');
                    continue;
                }
                if let Some((open_tag, close_tag)) = config.block_inline_tags.get(note) {
                    pending_config_heading = Some((open_tag.clone(), close_tag.clone()));
                    continue;
                }
            }

            fragment.push_str(&convert_inline(line, config));
            fragment.push('\n');
            continue;
        }

        if let Some(note) = page_break_note(trimmed)
            && let Some(close_tag) = config.block_close_tags.get(note)
        {
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
                    append_heading(&mut fragment, spec, content.trim_start(), config);
                }
                continue;
            }
            if let Some((open_tag, close_tag)) = generated_indent_block(note) {
                fragment.push_str(&open_tag);
                if !rest.trim().is_empty() {
                    fragment.push_str(&convert_inline(rest.trim_start(), config));
                    fragment.push('\n');
                }
                blocks.push(OpenBlock::Generated { close_tag });
                continue;
            }
            if let Some(spec) = block_heading_spec(note) {
                fragment.push('<');
                fragment.push_str(spec.element);
                fragment.push_str(" class=\"");
                fragment.push_str(spec.class_name);
                fragment.push_str("\">");
                if !rest.trim().is_empty() {
                    fragment.push_str(&convert_inline(rest.trim_start(), config));
                    fragment.push('\n');
                }
                blocks.push(OpenBlock::Hardcoded(spec));
                continue;
            }
            if let Some(tag) = config.block_single_tags.get(note) {
                fragment.push_str(tag);
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
                    fragment.push_str(open_tag);
                    fragment.push_str(&convert_inline(rest.trim_start(), config));
                    fragment.push_str(close_tag);
                    fragment.push('\n');
                }
                continue;
            }
            if let Some(open_tag) = config.block_open_tags.get(note) {
                fragment.push_str(open_tag);
                if !rest.trim().is_empty() {
                    fragment.push_str(&convert_inline(rest.trim_start(), config));
                    fragment.push('\n');
                }
                blocks.push(OpenBlock::Configured {
                    fallback_close_tag: fallback_close_tag(open_tag),
                });
                continue;
            }
        }

        append_line(&mut fragment, line, config);
    }

    while let Some(block) = blocks.pop() {
        match block {
            OpenBlock::Hardcoded(spec) => {
                fragment.push_str("</");
                fragment.push_str(spec.element);
                fragment.push_str(">\n");
            }
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
        append_heading(&mut fragment, spec, "", config);
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
            close_note: "",
        }),
        "大見出し" => Some(HeadingSpec {
            element: "h1",
            class_name: "font-1em50",
            close_note: "",
        }),
        "中見出し" => Some(HeadingSpec {
            element: "h2",
            class_name: "font-1em30",
            close_note: "",
        }),
        "小見出し" => Some(HeadingSpec {
            element: "h3",
            class_name: "font-1em10",
            close_note: "",
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
            format!("pt{wrapped} idt{}", indent.saturating_sub(wrapped)),
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

fn block_heading_spec(note: &str) -> Option<HeadingSpec> {
    match note {
        "ここから見出し" => Some(HeadingSpec {
            element: "h1",
            class_name: "font-1em50",
            close_note: "ここで見出し終わり",
        }),
        "ここから大見出し" => Some(HeadingSpec {
            element: "h1",
            class_name: "font-1em50",
            close_note: "ここで大見出し終わり",
        }),
        "ここから中見出し" => Some(HeadingSpec {
            element: "h2",
            class_name: "font-1em30",
            close_note: "ここで中見出し終わり",
        }),
        "ここから小見出し" => Some(HeadingSpec {
            element: "h3",
            class_name: "font-1em10",
            close_note: "ここで小見出し終わり",
        }),
        "ここから１字下げ" => Some(HeadingSpec {
            element: "div",
            class_name: "mt1",
            close_note: "ここで字下げ終わり",
        }),
        "ここから２字下げ" => Some(HeadingSpec {
            element: "div",
            class_name: "mt2",
            close_note: "ここで字下げ終わり",
        }),
        "ここから３字下げ" => Some(HeadingSpec {
            element: "div",
            class_name: "mt3",
            close_note: "ここで字下げ終わり",
        }),
        _ => None,
    }
}

fn fallback_close_tag(open_tag: &str) -> String {
    let tag_name = open_tag
        .strip_prefix('<')
        .and_then(|value| value.split([' ', '>']).next())
        .filter(|value| !value.is_empty())
        .unwrap_or("div");
    format!("</{tag_name}>")
}

fn append_heading(fragment: &mut String, spec: HeadingSpec, text: &str, config: &AozoraConfig) {
    fragment.push('<');
    fragment.push_str(spec.element);
    fragment.push_str(" class=\"");
    fragment.push_str(spec.class_name);
    fragment.push_str("\">");
    fragment.push_str(&convert_inline(text, config));
    fragment.push_str("</");
    fragment.push_str(spec.element);
    fragment.push_str(">\n");
}

fn append_line(fragment: &mut String, line: &str, config: &AozoraConfig) {
    if line.is_empty() {
        fragment.push_str("    <p><br/></p>\n");
    } else {
        fragment.push_str("    <p>");
        fragment.push_str(&convert_inline(line, config));
        fragment.push_str("</p>\n");
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
