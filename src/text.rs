use std::fmt;

use crate::config::AozoraConfig;
use encoding_rs::{Encoding, SHIFT_JIS, UTF_8};

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
    Ok(render_lines(input.lines(), config))
}

pub fn aozora_text_to_xhtml_sections(input: &str) -> Result<Vec<String>, TextError> {
    aozora_text_to_xhtml_sections_with_config(input, &AozoraConfig::default())
}

pub fn aozora_text_to_xhtml_sections_with_config(
    input: &str,
    config: &AozoraConfig,
) -> Result<Vec<String>, TextError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut sections = Vec::new();
    let mut current = Vec::new();

    for line in input.lines() {
        if let Some(note) = page_break_note(line)
            && config.page_break_notes.contains(note)
        {
            if config.split_page_breaks {
                if !current.is_empty() || sections.is_empty() {
                    sections.push(render_lines(current.iter().map(String::as_str), config));
                    current.clear();
                }
            }
            continue;
        }
        current.push(line.to_owned());
    }

    if !current.is_empty() || sections.is_empty() {
        sections.push(render_lines(current.iter().map(String::as_str), config));
    }

    Ok(sections)
}

#[derive(Clone, Copy)]
struct HeadingSpec {
    element: &'static str,
    class_name: &'static str,
    close_note: &'static str,
}

enum OpenBlock {
    Hardcoded(HeadingSpec),
    Configured { fallback_close_tag: String },
}

fn render_lines<'a>(lines: impl IntoIterator<Item = &'a str>, config: &AozoraConfig) -> String {
    let mut fragment = String::new();
    let mut has_line = false;
    let mut blocks: Vec<OpenBlock> = Vec::new();
    let mut pending_heading: Option<HeadingSpec> = None;
    let mut pending_config_heading: Option<(String, String)> = None;

    for line in lines {
        has_line = true;
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

                let closes_configured = matches!(blocks.last(), Some(OpenBlock::Configured { .. }));
                if closes_configured && let Some(close_tag) = config.block_close_tags.get(note) {
                    fragment.push_str(close_tag);
                    fragment.push('\n');
                    blocks.pop();
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
    fragment
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
        .and_then(|value| {
            value
                .split(|character| character == ' ' || character == '>')
                .next()
        })
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

fn convert_inline(input: &str, config: &AozoraConfig) -> String {
    let input = rewrite_suffix_notes(input, config);
    let input = rewrite_alternative_gaiji(&input, config);
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index] == '※'
            && let Some((end, replacement)) = parse_gaiji_note(&chars, index, config)
        {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if chars[index] == '※'
            && let Some((end, replacement)) = parse_unicode_note(&chars, index + 1)
        {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_unicode_note(&chars, index) {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_image_note(&chars, index) {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_inline_note(&chars, index, config) {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if chars[index] == '〔'
            && let Some(close) = find_closing_latin_bracket(&chars, index)
        {
            let inner = &chars[index + 1..close];
            if inner.iter().copied().all(is_half_space) {
                let separated = inner.iter().collect::<String>();
                let replacement = convert_latin(&separated, config);
                output.push_str(&escape_html(&replacement));
                index = close + 1;
                continue;
            }
        }

        if chars[index] == '｜'
            && let Some((open, close)) = find_ruby_bounds(&chars, index + 1)
        {
            let base = chars[index + 1..open].iter().collect::<String>();
            if !base.is_empty() {
                let reading = chars[open + 1..close].iter().collect::<String>();
                push_ruby(&mut output, &base, &reading);
                index = close + 1;
                continue;
            }
        }

        if chars[index] == '《'
            && let Some(close) = find_closing_ruby(&chars, index)
        {
            let mut base_start = index;
            while base_start > 0 && is_ruby_base(chars[base_start - 1]) {
                base_start -= 1;
            }
            if base_start < index {
                let base = chars[base_start..index].iter().collect::<String>();
                let escaped_base = escape_html(&base);
                if output.ends_with(&escaped_base) {
                    output.truncate(output.len() - escaped_base.len());
                    let reading = chars[index + 1..close].iter().collect::<String>();
                    push_ruby(&mut output, &base, &reading);
                    index = close + 1;
                    continue;
                }
            }
        }

        push_escaped_char(&mut output, chars[index]);
        index += 1;
    }

    output
}

fn rewrite_suffix_notes(input: &str, config: &AozoraConfig) -> String {
    let mut current = input.to_owned();

    loop {
        let chars = current.chars().collect::<Vec<_>>();
        let mut index = 0;
        let mut selected = None;

        while index < chars.len() {
            if let Some((end, target, suffix)) = suffix_note_at(&chars, index) {
                if let Some(rule) = config.suffix_notes.get(&suffix) {
                    let prefix = chars[..index].iter().collect::<String>();
                    if suffix_target_range(&prefix, &target).is_some() {
                        let target_length = target.chars().count();
                        let should_select = selected
                            .as_ref()
                            .is_none_or(|(_, _, _, _, _, length)| target_length > *length);
                        if should_select {
                            selected = Some((
                                index,
                                end,
                                target,
                                rule.start.clone(),
                                rule.end.clone(),
                                target_length,
                            ));
                        }
                    }
                }
                index = end;
            } else {
                index += 1;
            }
        }

        let Some((start, end, target, start_tag, end_tag, _)) = selected else {
            return current;
        };
        let prefix = chars[..start].iter().collect::<String>();
        let suffix = chars[end..].iter().collect::<String>();
        let (target_start, target_end) = suffix_target_range(&prefix, &target).unwrap();
        let start_note = format!("［＃{start_tag}］");
        let end_note = format!("［＃{end_tag}］");
        let mut rewritten = prefix;
        rewritten.insert_str(target_start, &start_note);
        rewritten.insert_str(target_end + start_note.len(), &end_note);
        rewritten.push_str(&suffix);
        current = rewritten;
    }
}

fn rewrite_alternative_gaiji(input: &str, config: &AozoraConfig) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < chars.len() {
        if let Some((end, note)) = gaiji_note_range(&chars, index) {
            if let Some(replacement) = config.gaiji_alternatives.get(&note) {
                output.push_str(replacement);
            } else {
                output.extend(chars[index..end].iter());
            }
            index = end;
        } else {
            output.push(chars[index]);
            index += 1;
        }
    }

    output
}

fn suffix_note_at(chars: &[char], start: usize) -> Option<(usize, String, String)> {
    if chars.get(start) != Some(&'［')
        || chars.get(start + 1) != Some(&'＃')
        || chars.get(start + 2) != Some(&'「')
    {
        return None;
    }
    let target_end = chars
        .iter()
        .enumerate()
        .skip(start + 3)
        .find_map(|(index, character)| (*character == '」').then_some(index))?;
    let close = chars
        .iter()
        .enumerate()
        .skip(target_end + 1)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let target = chars[start + 3..target_end].iter().collect::<String>();
    let suffix = chars[target_end + 1..close].iter().collect::<String>();
    (!target.is_empty() && !suffix.is_empty()).then_some((close + 1, target, suffix))
}

fn suffix_target_range(output: &str, target: &str) -> Option<(usize, usize)> {
    let indexed_chars = output.char_indices().collect::<Vec<_>>();
    let mut visible = Vec::new();
    let mut index = 0;

    while index < indexed_chars.len() {
        let (byte_index, character) = indexed_chars[index];
        if character == '［' && indexed_chars.get(index + 1).map(|(_, value)| *value) == Some('＃')
        {
            index = indexed_chars
                .iter()
                .enumerate()
                .skip(index + 2)
                .find_map(|(candidate, (_, value))| (*value == '］').then_some(candidate + 1))
                .unwrap_or(index + 1);
            continue;
        }
        if character == '｜' {
            index += 1;
            continue;
        }
        if character == '《' {
            index = indexed_chars
                .iter()
                .enumerate()
                .skip(index + 1)
                .find_map(|(candidate, (_, value))| (*value == '》').then_some(candidate + 1))
                .unwrap_or(index + 1);
            continue;
        }
        let end = byte_index + character.len_utf8();
        visible.push((character, byte_index, end));
        index += 1;
    }

    let target_chars = target.chars().collect::<Vec<_>>();
    if target_chars.is_empty() || target_chars.len() > visible.len() {
        return None;
    }
    let match_start = visible.len() - target_chars.len();
    if !visible[match_start..]
        .iter()
        .zip(target_chars)
        .all(|((character, _, _), target)| *character == target)
    {
        return None;
    }

    let mut start = visible[match_start].1;
    if start >= '｜'.len_utf8() && output[..start].ends_with('｜') {
        start -= '｜'.len_utf8();
    }
    let mut end = visible.last()?.2;
    if output[end..].starts_with('《')
        && let Some(ruby_end) = output[end..].find('》')
    {
        end += ruby_end + '》'.len_utf8();
    }
    Some((start, end))
}

fn parse_unicode_note(chars: &[char], start: usize) -> Option<(usize, String)> {
    if chars.get(start) != Some(&'［') || chars.get(start + 1) != Some(&'＃') {
        return None;
    }
    let close = chars
        .iter()
        .enumerate()
        .skip(start + 2)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let note = chars[start + 2..close].iter().collect::<String>();
    let upper = note.to_ascii_uppercase();
    let (marker, prefix_len) = [
        upper.find("U+").map(|index| (index, 2)),
        upper.find("UNICODE").map(|index| (index, 7)),
        upper.find("UCS").map(|index| (index, 3)),
    ]
    .into_iter()
    .flatten()
    .min_by_key(|(index, _)| *index)?;
    let (code, mut end) = parse_hex_code(&upper, marker + prefix_len)?;
    let mut replacement = String::from(char::from_u32(code)?);
    if upper.get(end..).is_some_and(|tail| tail.starts_with("-U+")) {
        let (variation, variation_end) = parse_hex_code(&upper, end + 1 + 2)?;
        replacement.push(char::from_u32(variation)?);
        end = variation_end;
    }
    let _ = end;
    Some((close + 1, replacement))
}

fn parse_gaiji_note(
    chars: &[char],
    start: usize,
    config: &AozoraConfig,
) -> Option<(usize, String)> {
    let (end, note) = gaiji_note_range(chars, start)?;
    Some((end, config.gaiji.get(&note)?.to_owned()))
}

fn gaiji_note_range(chars: &[char], start: usize) -> Option<(usize, String)> {
    if chars.get(start) != Some(&'※')
        || chars.get(start + 1) != Some(&'［')
        || chars.get(start + 2) != Some(&'＃')
    {
        return None;
    }
    let close = chars
        .iter()
        .enumerate()
        .skip(start + 3)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let note = chars[start..=close].iter().collect::<String>();
    Some((close + 1, note))
}

fn parse_hex_code(input: &str, start: usize) -> Option<(u32, usize)> {
    let end = input[start..]
        .char_indices()
        .find_map(|(offset, character)| (!character.is_ascii_hexdigit()).then_some(start + offset))
        .unwrap_or(input.len());
    if end == start {
        return None;
    }
    Some((u32::from_str_radix(&input[start..end], 16).ok()?, end))
}

fn parse_image_note(chars: &[char], start: usize) -> Option<(usize, String)> {
    let (end, path) = image_path_from_note(chars, start)?;
    let replacement = format!("<img src=\"../image/{}\" alt=\"\"/>", escape_html(&path));
    Some((end, replacement))
}

fn image_path_from_note(chars: &[char], start: usize) -> Option<(usize, String)> {
    if chars.get(start) != Some(&'［') || chars.get(start + 1) != Some(&'＃') {
        return None;
    }
    let close = chars
        .iter()
        .enumerate()
        .skip(start + 2)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let note = chars[start + 2..close].iter().collect::<String>();
    if !note.ends_with("入る") {
        return None;
    }
    let open_paren = note.find('（')?;
    let close_paren = note.rfind('）')?;
    if open_paren >= close_paren {
        return None;
    }
    let path = normalize_image_path(&note[open_paren + '（'.len_utf8()..close_paren])?;
    Some((close + 1, path))
}

fn normalize_image_path(path: &str) -> Option<String> {
    let path = path.trim().replace('\\', "/");
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return None;
        }
        parts.push(part);
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

pub fn image_references(input: &str) -> Vec<String> {
    let chars = input.chars().collect::<Vec<_>>();
    let mut references = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if let Some((end, path)) = image_path_from_note(&chars, index) {
            if !references.contains(&path) {
                references.push(path);
            }
            index = end;
        } else {
            index += 1;
        }
    }
    references
}

fn parse_inline_note(
    chars: &[char],
    start: usize,
    config: &AozoraConfig,
) -> Option<(usize, String)> {
    if chars.get(start) != Some(&'［') || chars.get(start + 1) != Some(&'＃') {
        return None;
    }
    let close = chars
        .iter()
        .enumerate()
        .skip(start + 2)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let note = chars[start + 2..close].iter().collect::<String>();
    let replacement = config.inline_notes.get(&note)?.clone();
    Some((close + 1, replacement))
}

fn find_ruby_bounds(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let open = chars
        .iter()
        .enumerate()
        .skip(start)
        .find_map(|(index, character)| (*character == '《').then_some(index))?;
    let close = find_closing_ruby(chars, open)?;
    Some((open, close))
}

fn find_closing_ruby(chars: &[char], open: usize) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(open + 1)
        .find_map(|(index, character)| (*character == '》').then_some(index))
}

fn find_closing_latin_bracket(chars: &[char], open: usize) -> Option<usize> {
    chars
        .iter()
        .enumerate()
        .skip(open + 1)
        .find_map(|(index, character)| (*character == '〕').then_some(index))
}

fn convert_latin(input: &str, config: &AozoraConfig) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < chars.len() {
        if let Some(replacement) =
            find_latin_replacement(&chars, index, 2, &config.latin_replacements)
        {
            output.push_str(replacement);
            index += 2;
            continue;
        }
        if let Some(replacement) =
            find_latin_replacement(&chars, index, 3, &config.latin_replacements)
        {
            output.push_str(replacement);
            index += 3;
            continue;
        }

        output.push(chars[index]);
        index += 1;
    }

    output
}

fn find_latin_replacement<'a>(
    chars: &[char],
    index: usize,
    length: usize,
    replacements: &'a std::collections::BTreeMap<String, String>,
) -> Option<&'a str> {
    let candidate = chars.get(index..index + length)?;
    replacements.iter().find_map(|(pattern, replacement)| {
        pattern
            .chars()
            .eq(candidate.iter().copied())
            .then_some(replacement.as_str())
    })
}

fn is_half_space(character: char) -> bool {
    (0x20..=0x02af).contains(&(character as u32))
}

fn is_ruby_base(character: char) -> bool {
    matches!(
        character as u32,
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff
    )
}

fn push_ruby(output: &mut String, base: &str, reading: &str) {
    output.push_str("<ruby>");
    output.push_str(&escape_html(base));
    output.push_str("<rt>");
    output.push_str(&escape_html(reading));
    output.push_str("</rt></ruby>");
}

pub fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        push_escaped_char(&mut escaped, character);
    }
    escaped
}

fn push_escaped_char(output: &mut String, character: char) {
    match character {
        '&' => output.push_str("&amp;"),
        '<' => output.push_str("&lt;"),
        '>' => output.push_str("&gt;"),
        '"' => output.push_str("&quot;"),
        '\'' => output.push_str("&apos;"),
        _ => output.push(character),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        aozora_text_to_xhtml_sections, decode_input, image_references, plain_text_to_xhtml,
    };
    use crate::config::AozoraConfig;
    use encoding_rs::SHIFT_JIS;

    #[test]
    fn escapes_text_and_preserves_empty_lines() {
        let output = plain_text_to_xhtml("A<&>\n\nB").unwrap();
        assert_eq!(
            output,
            "    <p>A&lt;&amp;&gt;</p>\n    <p><br/></p>\n    <p>B</p>\n"
        );
    }

    #[test]
    fn emits_a_placeholder_for_empty_input() {
        assert_eq!(plain_text_to_xhtml("").unwrap(), "    <p><br/></p>\n");
    }

    #[test]
    fn converts_explicit_and_implicit_ruby() {
        let output = plain_text_to_xhtml("｜漢字《かんじ》と青空《あおぞら》").unwrap();
        assert_eq!(
            output,
            "    <p><ruby>漢字<rt>かんじ</rt></ruby>と<ruby>青空<rt>あおぞら</rt></ruby></p>\n"
        );
    }

    #[test]
    fn splits_sections_at_page_break_tags() {
        let sections = aozora_text_to_xhtml_sections("前\n［＃改ページ］\n後").unwrap();
        assert_eq!(sections.len(), 2);
        assert!(sections[0].contains("<p>前</p>"));
        assert!(sections[1].contains("<p>後</p>"));
        assert!(!sections[0].contains("改ページ"));
        assert!(!sections[1].contains("改ページ"));
    }
    #[test]
    fn strips_utf8_bom_before_conversion() {
        let output = plain_text_to_xhtml("\u{feff}本文").unwrap();
        assert!(output.contains("<p>本文</p>"));
        assert!(!output.contains('\u{feff}'));
    }
    #[test]
    fn converts_representative_inline_notes() {
        let output = plain_text_to_xhtml(
            "［＃太字］太字［＃太字終わり］［＃縦中横］12［＃縦中横終わり］［＃改行］",
        )
        .unwrap();
        assert!(output.contains("<span class=\"bold\">太字</span>"));
        assert!(output.contains("<span class=\"tcy\">12</span><br/>"));
    }
    #[test]
    fn decodes_utf8_and_shift_jis_input() {
        let utf8 = decode_input("日本語".as_bytes(), None).unwrap();
        assert_eq!(utf8, "日本語");

        let (shift_jis, _, _) = SHIFT_JIS.encode("日本語");
        let decoded = decode_input(&shift_jis, Some("shift_jis")).unwrap();
        assert_eq!(decoded, "日本語");
    }
    #[test]
    fn converts_and_collects_image_notes() {
        let input = "画像［＃sample（fig/sample.png）入る］";
        let output = plain_text_to_xhtml(input).unwrap();
        assert!(output.contains("<img src=\"../image/fig/sample.png\" alt=\"\"/>"));
        assert_eq!(image_references(input), vec!["fig/sample.png"]);
    }
    #[test]
    fn renders_inline_and_block_headings() {
        let inline = plain_text_to_xhtml("［＃大見出し］章題\n本文").unwrap();
        assert!(inline.contains("<h1 class=\"font-1em50\">章題</h1>"));
        assert!(inline.contains("<p>本文</p>"));
        let closed_inline = plain_text_to_xhtml("［＃大見出し］章題［＃大見出し終わり］").unwrap();
        assert!(closed_inline.contains("<h1 class=\"font-1em50\">章題</h1>"));
        assert!(!closed_inline.contains("［＃大見出し終わり］"));

        let block =
            plain_text_to_xhtml("［＃ここから中見出し］\n章題\n［＃ここで中見出し終わり］\n本文")
                .unwrap();
        assert!(block.contains("<h2 class=\"font-1em30\">章題\n</h2>"));
        assert!(block.contains("<p>本文</p>"));
    }
    #[test]
    fn renders_basic_indent_blocks() {
        let output =
            plain_text_to_xhtml("［＃ここから１字下げ］\n字下げ本文\n［＃ここで字下げ終わり］")
                .unwrap();
        assert!(output.contains("<div class=\"mt1\">字下げ本文\n</div>"));
    }

    #[test]
    fn renders_configured_block_and_inline_block_tags() {
        let mut config = AozoraConfig::default();
        config.load_tag_text(
            "ここから太字\t<div class=\"bold\">\t\t1\n\
             ここで太字終わり\t</div>\t\t1\n\
             任意見出し\t<h1 class=\"custom\">\t</h1>\t1\n\
             空行\t<p><br/></p>\t\t1\n",
        );
        let output = super::plain_text_to_xhtml_with_config(
            "［＃ここから太字］\n本文\n［＃ここで太字終わり］\n\
             ［＃任意見出し］\n題名\n［＃空行］",
            &config,
        )
        .unwrap();
        assert!(output.contains("<div class=\"bold\">本文\n</div>"));
        assert!(output.contains("<h1 class=\"custom\">題名</h1>"));
        assert!(output.contains("<p><br/></p>"));
    }

    #[test]
    fn nests_configured_blocks_and_handles_single_tags_inside() {
        let mut config = AozoraConfig::default();
        config.load_tag_text(
            "ここから太字\t<div class=\"bold\">\t\t1\n\
             ここで太字終わり\t</div>\t\t1\n\
             空行\t<p><br/></p>\t\t1\n",
        );
        let output = super::plain_text_to_xhtml_with_config(
            "［＃ここから２字下げ］\n\
             ［＃ここから太字］\n\
             本文\n\
             ［＃空行］\n\
             ［＃ここで太字終わり］\n\
             ［＃ここで字下げ終わり］",
            &config,
        )
        .unwrap();
        assert!(
            output.contains(
                "<div class=\"mt2\"><div class=\"bold\">本文\n<p><br/></p>\n</div>\n</div>"
            )
        );
        assert!(!output.contains("［＃"));
    }
    #[test]
    fn nests_multiple_suffix_notes_on_the_same_target() {
        let mut config = AozoraConfig::default();
        config.load_suffix_text(
            "は太字\t太字\t太字終わり\nに傍点\t傍点\t傍点終わり\nに傍線\t傍線\t傍線終わり\n",
        );
        config.load_tag_text("傍線\t<span class=\"em-line\">\t\t\n傍線終わり\t</span>\t\t\n");
        let output = super::plain_text_to_xhtml_with_config(
            "青空［＃「青空」は太字］［＃「青空」に傍点］文庫《ぶんこ》［＃「青空文庫」に傍線］",
            &config,
        )
        .unwrap();
        assert!(output.contains(
            "<span class=\"em-line\"><span class=\"bold\"><span class=\"em-sesame\">青空"
        ));
        assert!(output.contains("</span></span><ruby>文庫<rt>ぶんこ</rt></ruby></span>"));
        assert!(!output.contains("［＃「青空"));
    }
    #[test]
    fn converts_unicode_and_ivs_gaiji_notes() {
        let output = plain_text_to_xhtml("※［＃U+845B］ ※［＃U+4E08-U+E0101］").unwrap();
        assert!(output.contains("葛"));
        assert!(output.contains("丈\u{e0101}"));
        assert!(!output.contains("［＃"));
    }
    #[test]
    fn applies_external_note_and_gaiji_configuration() {
        let mut config = AozoraConfig::default();
        config.load_tag_text("独自注記\t<span class=\"custom\">\t\t\n");
        config.load_utf_text("U+4E00\t\t一\t※［＃「外字」］\n");
        let output = super::plain_text_to_xhtml_with_config(
            "［＃独自注記］注記［＃傍点終わり］ ※［＃「外字」］",
            &config,
        )
        .unwrap();
        assert!(output.contains("<span class=\"custom\">注記</span>"));
        assert!(output.contains("一"));
    }

    #[test]
    fn converts_external_alternative_gaiji_before_inline_parsing() {
        let mut config = AozoraConfig::default();
        config.load_tag_text(
            "縦中横\t<span class=\"tcy\">\t\t\n縦中横終わり\t</span>\t\t\n小書き\t<span class=\"kogaki\">\t\t\n小書き終わり\t</span>\t\t\n",
        );
        config.load_alt_text(
            "\t\t［＃縦中横］!!!［＃縦中横終わり］\t※［＃感嘆符三つ］\n\t\t［＃小書き］こ［＃小書き終わり］\t※［＃小書き平仮名こ］\n",
        );
        let output = super::plain_text_to_xhtml_with_config(
            "※［＃感嘆符三つ］ ※［＃小書き平仮名こ］",
            &config,
        )
        .unwrap();
        assert!(output.contains("<span class=\"tcy\">!!!</span>"));
        assert!(output.contains("<span class=\"kogaki\">こ</span>"));
        assert!(!output.contains("※［＃"));
    }

    #[test]
    fn converts_external_latin_decomposition_inside_brackets() {
        let mut config = AozoraConfig::default();
        config.load_latin_text("A`\tÀ\nAE&\tÆ\n");
        let output =
            super::plain_text_to_xhtml_with_config("〔A` AE&〕 〔漢字〕", &config).unwrap();
        assert!(output.contains("<p>À Æ 〔漢字〕</p>"));
    }

    #[test]
    fn ini_page_break_setting_controls_section_split() {
        let ini = crate::config::IniSettings::parse("PageBreak=0").unwrap();
        let config = AozoraConfig::from_ini(ini);
        let sections =
            super::aozora_text_to_xhtml_sections_with_config("前\n［＃改ページ］\n後", &config)
                .unwrap();
        assert_eq!(sections.len(), 1);
        assert!(!sections[0].contains("改ページ"));
    }
    #[test]
    fn converts_external_suffix_notes_before_inline_parsing() {
        let mut config = AozoraConfig::default();
        config.load_suffix_text("に傍点\t傍点\t傍点終わり\n");
        let output = super::plain_text_to_xhtml_with_config(
            "青空［＃「青空」に傍点］\n｜青空《あおぞら》［＃「青空」に傍点］",
            &config,
        )
        .unwrap();
        assert!(output.contains("<span class=\"em-sesame\">青空</span>"));
        assert!(
            output.contains("<span class=\"em-sesame\"><ruby>青空<rt>あおぞら</rt></ruby></span>")
        );
    }
}
