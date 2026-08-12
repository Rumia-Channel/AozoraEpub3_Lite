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
        Self::parse(&fs::read_to_string(path)?)
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

pub struct AozoraConfig {
    pub ini: IniSettings,
    pub inline_notes: BTreeMap<String, String>,
    pub suffix_notes: BTreeMap<String, SuffixNoteRule>,
    pub gaiji: BTreeMap<String, String>,
    pub gaiji_alternatives: BTreeMap<String, String>,
    pub latin_replacements: BTreeMap<String, String>,
    pub character_replacements: BTreeMap<String, String>,
    pub block_open_tags: BTreeMap<String, String>,
    pub block_close_tags: BTreeMap<String, String>,
    pub block_inline_tags: BTreeMap<String, (String, String)>,
    pub block_single_tags: BTreeMap<String, String>,
    pub page_break_notes: BTreeSet<String>,
    pub page_middle_notes: BTreeSet<String>,
    pub page_bottom_notes: BTreeSet<String>,
    pub comment_print: bool,
    pub comment_convert: bool,
    pub title_page_write: bool,
    pub split_page_breaks: bool,
    pub auto_yoko: bool,
    pub auto_yoko_num1: bool,
    pub auto_yoko_num3: bool,
    pub auto_yoko_eq1: bool,
    pub auto_yoko_eq3: bool,
    pub dakuten_type: u8,
    pub print_ivs_bmp: bool,
    pub print_ivs_ssp: bool,
    pub vertical: bool,
}

impl Default for AozoraConfig {
    fn default() -> Self {
        let mut config = Self {
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
                ("見出し終わり".to_owned(), String::new()),
                ("大見出し終わり".to_owned(), String::new()),
                ("中見出し終わり".to_owned(), String::new()),
                ("小見出し終わり".to_owned(), String::new()),
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
                ("小書き".to_owned(), "<span class=\"kogaki\">".to_owned()),
                ("小書き終わり".to_owned(), "</span>".to_owned()),
            ]),
            suffix_notes: BTreeMap::new(),
            gaiji: BTreeMap::from([
                ("始め二重山括弧".to_owned(), "《".to_owned()),
                ("終わり二重山括弧".to_owned(), "》".to_owned()),
                ("始め角括弧".to_owned(), "［".to_owned()),
                ("終わり角括弧".to_owned(), "］".to_owned()),
                ("始め亀甲括弧".to_owned(), "〔".to_owned()),
                ("終わり亀甲括弧".to_owned(), "〕".to_owned()),
                ("縦線".to_owned(), "｜".to_owned()),
                ("井げた".to_owned(), "＃".to_owned()),
                ("米印".to_owned(), "※".to_owned()),
                ("くの字点".to_owned(), "／＼".to_owned()),
                ("くの字点（濁点付き）".to_owned(), "／″＼".to_owned()),
            ]),
            gaiji_alternatives: BTreeMap::new(),
            latin_replacements: BTreeMap::new(),
            character_replacements: BTreeMap::new(),
            block_open_tags: BTreeMap::from([(
                "ここから地付き".to_owned(),
                "<div class=\"btm\">".to_owned(),
            )]),
            block_close_tags: BTreeMap::from([
                ("ここで地付き終わり".to_owned(), "</div>".to_owned()),
                ("ここで地付き終り".to_owned(), "</div>".to_owned()),
            ]),
            block_inline_tags: BTreeMap::from([(
                "３字下げ".to_owned(),
                ("<div class=\"mt3\">".to_owned(), "</div>".to_owned()),
            )]),
            block_single_tags: BTreeMap::from([
                ("空行".to_owned(), "<p><br/></p>".to_owned()),
                ("区切り線".to_owned(), "<hr/>".to_owned()),
                ("ページの左右中央".to_owned(), String::new()),
            ]),
            page_break_notes: BTreeSet::from([
                "改丁".to_owned(),
                "改ページ".to_owned(),
                "改頁".to_owned(),
                "改段".to_owned(),
                "本文終わり".to_owned(),
                "ページの左右中央".to_owned(),
                "ページの左右中央に".to_owned(),
                "ページの左右中央から".to_owned(),
                "ページの天地左右中央".to_owned(),
                "ページの天地左右中央に".to_owned(),
                "改丁、ページの左右中央".to_owned(),
                "改丁、ページの左右中央に".to_owned(),
                "改ページ、ページの左右中央".to_owned(),
                "改ページ、ページの左右中央に".to_owned(),
                "ページ左".to_owned(),
                "ページの左".to_owned(),
                "ページ左寄せ".to_owned(),
                "ページの左寄せ".to_owned(),
            ]),
            page_middle_notes: BTreeSet::from([
                "ページの左右中央".to_owned(),
                "ページの左右中央に".to_owned(),
                "ページの左右中央から".to_owned(),
                "ページの天地左右中央".to_owned(),
                "ページの天地左右中央に".to_owned(),
                "改丁、ページの左右中央".to_owned(),
                "改丁、ページの左右中央に".to_owned(),
                "改ページ、ページの左右中央".to_owned(),
                "改ページ、ページの左右中央に".to_owned(),
            ]),
            page_bottom_notes: BTreeSet::from([
                "ページ左".to_owned(),
                "ページの左".to_owned(),
                "ページ左寄せ".to_owned(),
                "ページの左寄せ".to_owned(),
            ]),
            comment_print: false,
            comment_convert: false,
            title_page_write: false,
            split_page_breaks: true,
            auto_yoko: true,
            auto_yoko_num1: true,
            auto_yoko_num3: true,
            auto_yoko_eq1: true,
            auto_yoko_eq3: true,
            dakuten_type: 1,
            print_ivs_bmp: false,
            print_ivs_ssp: true,
            vertical: true,
        };
        config.load_tag_text(include_str!("../assets/aozora/chuki_tag.txt"));
        config.load_suffix_text(include_str!("../assets/aozora/chuki_tag_suf.txt"));
        config.load_utf_text(include_str!("../assets/aozora/chuki_utf.txt"));
        config.load_ivs_text(include_str!("../assets/aozora/chuki_ivs.txt"));
        config.load_alt_text(include_str!("../assets/aozora/chuki_alt.txt"));
        config.load_latin_text(include_str!("../assets/aozora/chuki_latin.txt"));
        config.load_replace_text(include_str!("../assets/aozora/replace.txt"));
        config
    }
}
impl AozoraConfig {
    pub fn from_ini(ini: IniSettings) -> Self {
        let split_page_breaks = ini.get_bool("PageBreak").unwrap_or(true);
        let comment_print = ini.get_bool("CommentPrint").unwrap_or(false);
        let comment_convert = ini.get_bool("CommentConvert").unwrap_or(false);
        let title_page_write = ini.get_bool("TitlePageWrite").unwrap_or(false);
        let auto_yoko = ini.get_bool("AutoYoko").unwrap_or(true);
        let auto_yoko_num1 = ini.get_bool("AutoYokoNum1").unwrap_or(true);
        let auto_yoko_num3 = ini.get_bool("AutoYokoNum3").unwrap_or(true);
        let auto_yoko_eq1 = ini.get_bool("AutoYokoEQ1").unwrap_or(true);
        let auto_yoko_eq3 = ini.get_bool("AutoYokoEQ3").unwrap_or(true);
        let dakuten_type = ini
            .get("DakutenType")
            .and_then(|value| value.parse::<u8>().ok())
            .filter(|value| *value <= 2)
            .unwrap_or(1);
        let print_ivs_bmp = ini.get_bool("IvsBMP").unwrap_or(false);
        let print_ivs_ssp = ini.get_bool("IvsSSP").unwrap_or(true);
        let vertical = ini.get_bool("Vertical").unwrap_or(true);
        Self {
            ini,
            split_page_breaks,
            comment_print,
            title_page_write,
            comment_convert,
            auto_yoko,
            auto_yoko_num1,
            auto_yoko_num3,
            auto_yoko_eq1,
            auto_yoko_eq3,
            dakuten_type,
            print_ivs_bmp,
            print_ivs_ssp,
            vertical,
            ..Self::default()
        }
    }

    pub fn load(
        config_dir: Option<&Path>,
        preset_path: Option<&Path>,
    ) -> Result<Self, ConfigError> {
        match config_dir {
            Some(directory) => Self::load_from_dirs(&[directory], preset_path),
            None => Self::load_from_dirs(&[], preset_path),
        }
    }

    pub fn load_from_dirs(
        config_dirs: &[&Path],
        preset_path: Option<&Path>,
    ) -> Result<Self, ConfigError> {
        let mut config = match preset_path {
            Some(path) => Self::from_ini(IniSettings::load(path)?),
            None => Self::default(),
        };
        for directory in config_dirs {
            config.load_directory(directory)?;
        }
        Ok(config)
    }

    fn load_directory(&mut self, directory: &Path) -> Result<(), ConfigError> {
        if !directory.is_dir() {
            return Err(ConfigError::InvalidPath(directory.display().to_string()));
        }
        self.load_optional_file(directory.join("chuki_tag.txt"), Self::load_tag_text)?;
        self.load_optional_file(directory.join("custom_chuki_tag.txt"), Self::load_tag_text)?;
        self.load_optional_file(directory.join("chuki_tag_suf.txt"), Self::load_suffix_text)?;
        self.load_optional_file(directory.join("chuki_utf.txt"), Self::load_utf_text)?;
        self.load_optional_file(directory.join("chuki_ivs.txt"), Self::load_ivs_text)?;
        self.load_optional_file(directory.join("chuki_alt.txt"), Self::load_alt_text)?;
        self.load_optional_file(directory.join("chuki_latin.txt"), Self::load_latin_text)?;
        self.load_optional_file(directory.join("replace.txt"), Self::load_replace_text)?;
        Ok(())
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
            let flag = fields
                .iter()
                .skip(1)
                .map(|value| value.trim())
                .find_map(|value| {
                    let mut chars = value.chars();
                    let flag = chars.next()?;
                    (chars.next().is_none()
                        && matches!(flag, '1' | '2' | '3' | 'P' | 'M' | 'L' | 'K'))
                    .then_some(flag)
                });
            match flag {
                Some('P') => {
                    self.page_break_notes.insert(note.to_owned());
                }
                Some('M') => {
                    self.page_break_notes.insert(note.to_owned());
                    self.page_middle_notes.insert(note.to_owned());
                }
                Some('L') => {
                    self.page_break_notes.insert(note.to_owned());
                    self.page_bottom_notes.insert(note.to_owned());
                }
                _ => {}
            }
            if flag == Some('1') {
                let tag = fields
                    .get(1)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty());
                let close_tag = fields
                    .get(2)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty());
                if let Some(tag) = tag {
                    if tag.starts_with("</") {
                        self.block_close_tags
                            .insert(note.to_owned(), tag.to_owned());
                    } else if let Some(close_tag) = close_tag {
                        self.block_inline_tags
                            .insert(note.to_owned(), (tag.to_owned(), close_tag.to_owned()));
                    } else if tag.ends_with("/>") || tag.contains("</") {
                        self.block_single_tags
                            .insert(note.to_owned(), tag.to_owned());
                    } else {
                        self.block_open_tags.insert(note.to_owned(), tag.to_owned());
                    }
                }
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
    pub fn load_alt_text(&mut self, input: &str) {
        load_gaiji_rows(&mut self.gaiji_alternatives, input);
    }

    pub fn load_latin_text(&mut self, input: &str) {
        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            let Some(pattern) = fields
                .first()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(replacement) = fields
                .get(1)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            self.latin_replacements
                .insert(pattern.to_owned(), replacement.to_owned());
        }
    }

    pub fn load_replace_text(&mut self, input: &str) {
        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            let Some(from) = fields
                .first()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(to) = fields
                .get(1)
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if from.chars().count() <= 2 {
                self.character_replacements
                    .insert(from.to_owned(), to.to_owned());
            }
        }
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
        if character.is_empty() || !note.starts_with('※') {
            continue;
        }
        target
            .entry(note.to_owned())
            .or_insert_with(|| character.to_owned());
        if let Some((prefix, _)) = note.split_once('、') {
            target
                .entry(format!("{prefix}］"))
                .or_insert_with(|| character.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{AozoraConfig, IniSettings};

    #[test]
    fn parses_flat_and_sectioned_ini_values() {
        let ini = IniSettings::parse("PageBreak=0\n[layout]\nLineHeight = 1.8\n").unwrap();
        assert_eq!(ini.get("PageBreak"), Some("0"));
        assert_eq!(ini.get("layout.LineHeight"), Some("1.8"));
        assert_eq!(ini.get_bool("PageBreak"), Some(false));
    }

    #[test]
    fn reads_title_page_write_setting() {
        assert!(!AozoraConfig::default().title_page_write);
        let config = AozoraConfig::from_ini(IniSettings::parse("TitlePageWrite=1\n").unwrap());
        assert!(config.title_page_write);
    }

    #[test]
    fn loads_tag_and_gaiji_rows() {
        let mut config = AozoraConfig::default();
        config.load_tag_text(
            "注記\t<span class=\"note\">\t\tP\n\
             改丁\t\tP\n\
             ページ中央\t\tM\n",
        );
        config.load_utf_text("U+4E00\t\t一\t※［＃「一」］\n");
        assert_eq!(
            config.inline_notes.get("注記"),
            Some(&"<span class=\"note\">".to_owned())
        );
        assert!(config.page_break_notes.contains("注記"));
        assert!(config.page_break_notes.contains("改丁"));
        assert!(config.page_break_notes.contains("ページ中央"));
        assert_eq!(config.gaiji.get("※［＃「一」］"), Some(&"一".to_owned()));
    }

    #[test]
    fn loads_standard_and_overlay_directories_in_order() {
        let root = std::env::temp_dir().join(format!(
            "aozora-epub3-config-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let standard = root.join("standard");
        let overlay = root.join("overlay");
        fs::create_dir_all(&standard).unwrap();
        fs::create_dir_all(&overlay).unwrap();
        fs::write(
            standard.join("chuki_tag.txt"),
            "標準注記\t<span class=\"standard\">\n\
             共通注記\t<span class=\"standard\">\n",
        )
        .unwrap();
        fs::write(
            overlay.join("custom_chuki_tag.txt"),
            "追加注記\t<span class=\"overlay\">\n\
             共通注記\t<span class=\"overlay\">\n",
        )
        .unwrap();

        let config =
            AozoraConfig::load_from_dirs(&[standard.as_path(), overlay.as_path()], None).unwrap();
        assert_eq!(
            config.inline_notes.get("標準注記"),
            Some(&"<span class=\"standard\">".to_owned())
        );
        assert_eq!(
            config.inline_notes.get("追加注記"),
            Some(&"<span class=\"overlay\">".to_owned())
        );
        assert_eq!(
            config.inline_notes.get("共通注記"),
            Some(&"<span class=\"overlay\">".to_owned())
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loads_suffix_note_rules_and_aliases() {
        let mut config = AozoraConfig::default();
        config.load_suffix_text("に傍点\t傍点\t傍点終わり\n");
        let rule = config.suffix_notes.get("に傍点").unwrap();
        assert_eq!(rule.start, "傍点");
        assert_eq!(rule.end, "傍点終わり");
    }

    #[test]
    fn loads_alternative_gaiji_rows() {
        let mut config = AozoraConfig::default();
        config.load_alt_text("\t\t［＃縦中横］!!!［＃縦中横終わり］\t※［＃感嘆符三つ］\n");
        assert_eq!(
            config.gaiji_alternatives.get("※［＃感嘆符三つ］"),
            Some(&"［＃縦中横］!!!［＃縦中横終わり］".to_owned())
        );
    }

    #[test]
    fn loads_block_tag_definitions() {
        let mut config = AozoraConfig::default();
        config.load_tag_text(
            "ここから太字\t<div class=\"bold\">\t\t1\n\
             ここで太字終わり\t</div>\t\t1\n\
             見出し\t<h1 class=\"title\">\t</h1>\t1\n\
             空行\t<p><br/></p>\t\t1\n",
        );
        assert_eq!(
            config.block_open_tags.get("ここから太字"),
            Some(&"<div class=\"bold\">".to_owned())
        );
        assert_eq!(
            config.block_close_tags.get("ここで太字終わり"),
            Some(&"</div>".to_owned())
        );
        assert_eq!(
            config.block_inline_tags.get("見出し"),
            Some(&("<h1 class=\"title\">".to_owned(), "</h1>".to_owned()))
        );
        assert_eq!(
            config.block_single_tags.get("空行"),
            Some(&"<p><br/></p>".to_owned())
        );
    }
    #[test]
    fn loads_latin_replacement_rows() {
        let mut config = AozoraConfig::default();
        config.load_latin_text("A`\tÀ\t164\t8883\nAE&\tÆ\n");
        assert_eq!(config.latin_replacements.get("A`"), Some(&"À".to_owned()));
        assert_eq!(config.latin_replacements.get("AE&"), Some(&"Æ".to_owned()));
    }
}
