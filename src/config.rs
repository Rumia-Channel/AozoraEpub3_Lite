use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    InvalidIni { line: usize, message: String },
    InvalidPath(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "configuration I/O error: {error}"),
            Self::InvalidIni { line, message } => {
                write!(formatter, "invalid ini at line {line}: {message}")
            }
            Self::InvalidPath(path) => {
                write!(formatter, "configuration path is not a directory: {path}")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IniSettings {
    values: BTreeMap<String, String>,
}

impl IniSettings {
    pub fn parse(input: &str) -> Result<Self, ConfigError> {
        let mut values = BTreeMap::new();
        let mut section = None;

        for (line_index, line) in input.lines().enumerate() {
            let line_number = line_index + 1;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }
            if trimmed.starts_with('[') {
                let section_name = trimmed
                    .strip_prefix('[')
                    .and_then(|value| value.strip_suffix(']'));
                let Some(section_name) = section_name
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return Err(ConfigError::InvalidIni {
                        line: line_number,
                        message: "section name is empty or not closed".to_owned(),
                    });
                };
                section = Some(section_name.to_owned());
                continue;
            }

            let Some((key, value)) = trimmed.split_once('=') else {
                return Err(ConfigError::InvalidIni {
                    line: line_number,
                    message: "expected key=value".to_owned(),
                });
            };
            let key = key.trim();
            if key.is_empty() {
                return Err(ConfigError::InvalidIni {
                    line: line_number,
                    message: "key is empty".to_owned(),
                });
            }
            let key = match &section {
                Some(section) => format!("{section}.{key}"),
                None => key.to_owned(),
            };
            values.insert(key, value.trim().to_owned());
        }

        Ok(Self { values })
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        Ok(Self::parse(&fs::read_to_string(path)?)?)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key)?.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuffixNoteRule {
    pub start: String,
    pub end: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AozoraConfig {
    pub ini: IniSettings,
    pub inline_notes: BTreeMap<String, String>,
    pub suffix_notes: BTreeMap<String, SuffixNoteRule>,
    pub gaiji: BTreeMap<String, String>,
    pub page_break_notes: BTreeSet<String>,
    pub split_page_breaks: bool,
}

impl Default for AozoraConfig {
    fn default() -> Self {
        Self {
            ini: IniSettings::default(),
            inline_notes: BTreeMap::from([
                ("傍点".to_owned(), "<span class=\"em-sesame\">".to_owned()),
                ("傍点終わり".to_owned(), "</span>".to_owned()),
                ("太字".to_owned(), "<span class=\"bold\">".to_owned()),
                ("太字終わり".to_owned(), "</span>".to_owned()),
                ("斜体".to_owned(), "<span class=\"italic\">".to_owned()),
                ("斜体終わり".to_owned(), "</span>".to_owned()),
                ("ゴシック体".to_owned(), "<span class=\"gfont\">".to_owned()),
                ("ゴシック体終わり".to_owned(), "</span>".to_owned()),
                ("縦中横".to_owned(), "<span class=\"tcy\">".to_owned()),
                ("縦中横終わり".to_owned(), "</span>".to_owned()),
                ("割り注".to_owned(), "<span class=\"wrc\">".to_owned()),
                (
                    "ここから割り注".to_owned(),
                    "<span class=\"wrc\">".to_owned(),
                ),
                ("割り注終わり".to_owned(), "</span>".to_owned()),
                ("ここで割り注終わり".to_owned(), "</span>".to_owned()),
                ("改行".to_owned(), "<br/>".to_owned()),
                ("行右小書き".to_owned(), "<span class=\"super\">".to_owned()),
                (
                    "上付き小文字".to_owned(),
                    "<span class=\"super\">".to_owned(),
                ),
                ("行右小書き終わり".to_owned(), "</span>".to_owned()),
                ("上付き小文字終わり".to_owned(), "</span>".to_owned()),
                ("行左小書き".to_owned(), "<span class=\"sub\">".to_owned()),
                ("下付き小文字".to_owned(), "<span class=\"sub\">".to_owned()),
                ("行左小書き終わり".to_owned(), "</span>".to_owned()),
                ("下付き小文字終わり".to_owned(), "</span>".to_owned()),
            ]),
            suffix_notes: BTreeMap::new(),
            gaiji: BTreeMap::new(),
            page_break_notes: BTreeSet::from(["改ページ".to_owned()]),
            split_page_breaks: true,
        }
    }
}

impl AozoraConfig {
    pub fn from_ini(ini: IniSettings) -> Self {
        let split_page_breaks = ini.get_bool("PageBreak").unwrap_or(true);
        Self {
            ini,
            split_page_breaks,
            ..Self::default()
        }
    }

    pub fn load(
        config_dir: Option<&Path>,
        preset_path: Option<&Path>,
    ) -> Result<Self, ConfigError> {
        let mut config = match preset_path {
            Some(path) => Self::from_ini(IniSettings::load(path)?),
            None => Self::default(),
        };

        if let Some(directory) = config_dir {
            if !directory.is_dir() {
                return Err(ConfigError::InvalidPath(directory.display().to_string()));
            }
            config.load_optional_file(directory.join("chuki_tag.txt"), Self::load_tag_text)?;
            config
                .load_optional_file(directory.join("chuki_tag_suf.txt"), Self::load_suffix_text)?;
            config.load_optional_file(directory.join("chuki_utf.txt"), Self::load_utf_text)?;
            config.load_optional_file(directory.join("chuki_ivs.txt"), Self::load_ivs_text)?;
        }

        Ok(config)
    }

    pub fn load_tag_text(&mut self, input: &str) {
        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            let Some(note) = fields
                .first()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if let Some(tag) = fields
                .get(1)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                self.inline_notes.insert(note.to_owned(), tag.to_owned());
            }
            if fields
                .iter()
                .skip(1)
                .any(|value| value.trim().contains('P'))
            {
                self.page_break_notes.insert(note.to_owned());
            }
        }
    }

    pub fn load_suffix_text(&mut self, input: &str) {
        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            let Some(key) = fields
                .first()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(start) = fields
                .get(1)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(end) = fields
                .get(2)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let rule = SuffixNoteRule {
                start: start.to_owned(),
                end: end.to_owned(),
            };
            self.suffix_notes.insert(key.to_owned(), rule.clone());
            if let Some(alias) = fields
                .get(3)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            {
                self.suffix_notes.insert(format!("{alias}{key}"), rule);
            }
        }
    }

    pub fn load_utf_text(&mut self, input: &str) {
        load_gaiji_rows(&mut self.gaiji, input);
    }

    pub fn load_ivs_text(&mut self, input: &str) {
        load_gaiji_rows(&mut self.gaiji, input);
    }

    fn load_optional_file(
        &mut self,
        path: impl AsRef<Path>,
        loader: fn(&mut Self, &str),
    ) -> Result<(), ConfigError> {
        let path = path.as_ref();
        if path.is_file() {
            loader(self, &fs::read_to_string(path)?);
        }
        Ok(())
    }
}

fn load_gaiji_rows(target: &mut BTreeMap<String, String>, input: &str) {
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() < 4 {
            continue;
        }
        let character = fields[2].trim();
        let note = fields[3].trim();
        if !character.is_empty() && note.starts_with('※') {
            target.insert(note.to_owned(), character.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AozoraConfig, IniSettings};

    #[test]
    fn parses_flat_and_sectioned_ini_values() {
        let ini = IniSettings::parse("PageBreak=0\n[layout]\nLineHeight = 1.8\n").unwrap();
        assert_eq!(ini.get("PageBreak"), Some("0"));
        assert_eq!(ini.get("layout.LineHeight"), Some("1.8"));
        assert_eq!(ini.get_bool("PageBreak"), Some(false));
    }

    #[test]
    fn loads_tag_and_gaiji_rows() {
        let mut config = AozoraConfig::default();
        config.load_tag_text("注記\t<span class=\"note\">\t\tP\n改丁\t\tP\n");
        config.load_utf_text("U+4E00\t\t一\t※［＃「一」］\n");
        assert_eq!(
            config.inline_notes.get("注記"),
            Some(&"<span class=\"note\">".to_owned())
        );
        assert!(config.page_break_notes.contains("注記"));
        assert!(config.page_break_notes.contains("改丁"));
        assert_eq!(config.gaiji.get("※［＃「一」］"), Some(&"一".to_owned()));
    }

    #[test]
    fn loads_suffix_note_rules_and_aliases() {
        let mut config = AozoraConfig::default();
        config.load_suffix_text("に傍点\t傍点\t傍点終わり\n");
        let rule = config.suffix_notes.get("に傍点").unwrap();
        assert_eq!(rule.start, "傍点");
        assert_eq!(rule.end, "傍点終わり");
    }
}
