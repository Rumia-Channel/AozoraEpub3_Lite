use std::fmt;

use encoding_rs::{Encoding, SHIFT_JIS, UTF_8};
const PAGE_BREAK_TAG: &str = "［＃改ページ］";

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
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    Ok(render_lines(input.lines()))
}

pub fn aozora_text_to_xhtml_sections(input: &str) -> Result<Vec<String>, TextError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut sections = Vec::new();
    let mut current = Vec::new();

    for line in input.lines() {
        if line.trim() == PAGE_BREAK_TAG {
            if !current.is_empty() || sections.is_empty() {
                sections.push(render_lines(current.iter().map(String::as_str)));
                current.clear();
            }
        } else {
            current.push(line.to_owned());
        }
    }

    if !current.is_empty() || sections.is_empty() {
        sections.push(render_lines(current.iter().map(String::as_str)));
    }

    Ok(sections)
}

#[derive(Clone, Copy)]
struct HeadingSpec {
    element: &'static str,
    class_name: &'static str,
    close_note: &'static str,
}

fn render_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    let mut fragment = String::new();
    let mut has_line = false;
    let mut block: Option<HeadingSpec> = None;
    let mut pending_heading: Option<HeadingSpec> = None;

    for line in lines {
        has_line = true;
        let trimmed = line.trim();

        if let Some(spec) = block {
            if trimmed == format!("［＃{}］", spec.close_note) {
                fragment.push_str("</");
                fragment.push_str(spec.element);
                fragment.push_str(">\n");
                block = None;
            } else {
                fragment.push_str(&convert_inline(line));
                fragment.push('\n');
            }
            continue;
        }

        if let Some(spec) = pending_heading.take() {
            append_heading(&mut fragment, spec, line);
            continue;
        }

        if let Some((note, rest)) = heading_note_at_start(line) {
            if let Some(spec) = heading_spec(note) {
                if rest.trim().is_empty() {
                    pending_heading = Some(spec);
                } else {
                    append_heading(&mut fragment, spec, rest.trim_start());
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
                    fragment.push_str(&convert_inline(rest.trim_start()));
                    fragment.push('\n');
                }
                block = Some(spec);
                continue;
            }
        }

        append_line(&mut fragment, line);
    }

    if let Some(spec) = block {
        fragment.push_str("</");
        fragment.push_str(spec.element);
        fragment.push_str(">\n");
    } else if let Some(spec) = pending_heading {
        append_heading(&mut fragment, spec, "");
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
        _ => None,
    }
}

fn append_heading(fragment: &mut String, spec: HeadingSpec, text: &str) {
    fragment.push('<');
    fragment.push_str(spec.element);
    fragment.push_str(" class=\"");
    fragment.push_str(spec.class_name);
    fragment.push_str("\">");
    fragment.push_str(&convert_inline(text));
    fragment.push_str("</");
    fragment.push_str(spec.element);
    fragment.push_str(">\n");
}

fn append_line(fragment: &mut String, line: &str) {
    if line.is_empty() {
        fragment.push_str("    <p><br/></p>\n");
    } else {
        fragment.push_str("    <p>");
        fragment.push_str(&convert_inline(line));
        fragment.push_str("</p>\n");
    }
}

fn convert_inline(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0;

    while index < chars.len() {
        if let Some((end, replacement)) = parse_image_note(&chars, index) {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_inline_note(&chars, index) {
            output.push_str(replacement);
            index = end;
            continue;
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

fn parse_inline_note(chars: &[char], start: usize) -> Option<(usize, &'static str)> {
    if chars.get(start) != Some(&'［') || chars.get(start + 1) != Some(&'＃') {
        return None;
    }
    let close = chars
        .iter()
        .enumerate()
        .skip(start + 2)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let note = chars[start + 2..close].iter().collect::<String>();
    let replacement = match note.as_str() {
        "傍点" => "<span class=\"em-sesame\">",
        "傍点終わり" => "</span>",
        "太字" => "<span class=\"bold\">",
        "太字終わり" => "</span>",
        "斜体" => "<span class=\"italic\">",
        "斜体終わり" => "</span>",
        "ゴシック体" => "<span class=\"gfont\">",
        "ゴシック体終わり" => "</span>",
        "縦中横" => "<span class=\"tcy\">",
        "縦中横終わり" => "</span>",
        "割り注" | "ここから割り注" => "<span class=\"wrc\">",
        "割り注終わり" | "ここで割り注終わり" => "</span>",
        "改行" => "<br/>",
        "行右小書き" | "上付き小文字" => "<span class=\"super\">",
        "行右小書き終わり" | "上付き小文字終わり" => "</span>",
        "行左小書き" | "下付き小文字" => "<span class=\"sub\">",
        "行左小書き終わり" | "下付き小文字終わり" => "</span>",
        _ => return None,
    };
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

        let block =
            plain_text_to_xhtml("［＃ここから中見出し］\n章題\n［＃ここで中見出し終わり］\n本文")
                .unwrap();
        assert!(block.contains("<h2 class=\"font-1em30\">章題\n</h2>"));
        assert!(block.contains("<p>本文</p>"));
    }
}
