use std::fmt;

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
    let mut fragment = String::new();

    for line in input.lines() {
        if line.is_empty() {
            fragment.push_str("    <p><br/></p>\n");
        } else {
            fragment.push_str("    <p>");
            fragment.push_str(&escape_html(line));
            fragment.push_str("</p>\n");
        }
    }

    if input.is_empty() {
        fragment.push_str("    <p><br/></p>\n");
    }

    Ok(fragment)
}

pub fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::plain_text_to_xhtml;

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
}
