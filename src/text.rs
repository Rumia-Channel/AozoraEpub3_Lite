use std::fmt;

const PAGE_BREAK_TAG: &str = "［＃改ページ］";

#[derive(Debug, Eq, PartialEq)]
pub enum TextError {
    InvalidInput,
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput => write!(f, "input text is not valid UTF-8"),
        }
    }
}

impl std::error::Error for TextError {}

pub fn plain_text_to_xhtml(input: &str) -> Result<String, TextError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    Ok(render_lines(input.lines()))
}

pub fn aozora_text_to_xhtml_sections(input: &str) -> Result<Vec<String>, TextError> {
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let mut current = String::new();
    let mut sections = Vec::new();

    for line in input.lines() {
        if line.trim() == PAGE_BREAK_TAG {
            if !current.is_empty() || sections.is_empty() {
                sections.push(std::mem::take(&mut current));
            }
        } else {
            append_line(&mut current, line);
        }
    }

    if !current.is_empty() || sections.is_empty() {
        sections.push(current);
    }

    for section in &mut sections {
        if section.is_empty() {
            section.push_str("    <p><br/></p>\n");
        }
    }

    Ok(sections)
}

fn render_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    let mut fragment = String::new();
    let mut has_line = false;
    for line in lines {
        has_line = true;
        append_line(&mut fragment, line);
    }
    if !has_line {
        fragment.push_str("    <p><br/></p>\n");
    }
    fragment
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
    use super::{aozora_text_to_xhtml_sections, plain_text_to_xhtml};

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
}
