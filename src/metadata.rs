//! AozoraEpub3-compatible book metadata detection.
//!
//! Ports the Java reference behavior (`BookInfo.setMetaInfo` and the
//! leading-line collection in `AozoraEpub3Converter.getBookInfo`):
//!
//! * TitleType 0..4 plus NONE, with publisher-first support,
//! * the 0-6 leading non-empty line layouts (title/subtitle/author lines),
//! * post-processing of title and creator (ruby removal, chuki removal,
//!   symbol collapsing),
//! * `[creator] title` file name parsing.
//!
//! All line indices returned are 0-based over `input.lines()`, so callers
//! can omit or mark the metadata lines when rendering.

use std::collections::BTreeSet;

/// Page-break notes that terminate the leading metadata block. The same
/// default set as the converter configuration (`page_break_notes`).
pub const DEFAULT_PAGE_BREAK_NOTES: &[&str] = &[
    "改丁",
    "改ページ",
    "改頁",
    "改段",
    "本文終わり",
    "ページの左右中央",
    "ページの左右中央に",
    "ページの左右中央から",
    "ページの天地左右中央",
    "ページの天地左右中央に",
    "改丁、ページの左右中央",
    "改丁、ページの左右中央に",
    "改ページ、ページの左右中央",
    "改ページ、ページの左右中央に",
    "ページ左",
    "ページの左",
    "ページ左寄せ",
    "ページの左寄せ",
];

/// Comment block marker: a line of at least 50 ASCII dashes.
const COMMENT_MARKER: &str = "--------------------------------------------------";

/// 表題種別, matching `BookInfo.TitleType` enum order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TitleType {
    /// 0: 表題 → 著者名 (title first, author after).
    TitleAuthor = 0,
    /// 1: 著者名 → 表題 (author first).
    AuthorTitle = 1,
    /// 2: 表題 → 著者名, subtitle preferred.
    SubtitleAuthor = 2,
    /// 3: 表題のみ (title only, one line).
    TitleOnly = 3,
    /// 4: 表題+著者のみ (title + author, two lines).
    TitleAuthorOnly = 4,
    /// 5: なし (no text metadata).
    None = 5,
}

impl TitleType {
    /// All title types in index order.
    pub const ALL: [Self; 6] = [
        Self::TitleAuthor,
        Self::AuthorTitle,
        Self::SubtitleAuthor,
        Self::TitleOnly,
        Self::TitleAuthorOnly,
        Self::None,
    ];

    /// Resolves a CLI/index value (0..=5) to a title type.
    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    /// The index used by the `-t` option.
    pub fn index(self) -> usize {
        self as usize
    }

    /// True when the title line comes before the creator line.
    pub fn title_first(self) -> bool {
        matches!(
            self,
            Self::TitleAuthor | Self::SubtitleAuthor | Self::TitleOnly | Self::TitleAuthorOnly
        )
    }

    /// True when a title is extracted from the text.
    pub fn has_title(self) -> bool {
        !matches!(self, Self::None)
    }

    /// True when a creator is extracted from the text.
    pub fn has_author(self) -> bool {
        matches!(
            self,
            Self::TitleAuthor | Self::AuthorTitle | Self::SubtitleAuthor | Self::TitleAuthorOnly
        )
    }
}

/// Detected book metadata with source line indices (0-based).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BookMeta {
    pub title: Option<String>,
    pub creator: Option<String>,
    pub publisher: Option<String>,
    pub title_line: Option<usize>,
    pub creator_line: Option<usize>,
    pub publisher_line: Option<usize>,
    pub subtitle_line: Option<usize>,
    /// First line of the leading metadata block.
    pub meta_line_start: Option<usize>,
    /// Last metadata line consumed (inclusive); lines in
    /// `meta_line_start..=title_end_line` are the metadata block.
    pub title_end_line: Option<usize>,
}

/// Detects title/creator/publisher from the leading lines of `input`.
pub fn detect_meta(input: &str, title_type: TitleType, publisher_first: bool) -> BookMeta {
    let page_breaks = DEFAULT_PAGE_BREAK_NOTES
        .iter()
        .map(|value| (*value).to_owned())
        .collect();
    detect_meta_with_page_breaks(input, title_type, publisher_first, &page_breaks)
}
/// Detects metadata after applying the configured Aozora gaiji replacements.
///
/// The Java converter resolves gaiji notes before collecting title lines, so
/// special escaped characters such as `※［＃始め二重山括弧］` remain visible in
/// the title. The line layout and indices are preserved by this conversion.
pub fn detect_meta_with_gaiji(
    input: &str,
    title_type: TitleType,
    publisher_first: bool,
    gaiji: &std::collections::BTreeMap<String, String>,
) -> BookMeta {
    let converted = convert_gaiji_notes(input, gaiji);
    detect_meta(&converted, title_type, publisher_first)
}

fn convert_gaiji_notes(input: &str, gaiji: &std::collections::BTreeMap<String, String>) -> String {
    const NOTE_PREFIX: &str = "※［＃";
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(relative_start) = input[cursor..].find(NOTE_PREFIX) {
        let start = cursor + relative_start;
        output.push_str(&input[cursor..start]);
        let note_start = start + NOTE_PREFIX.len();
        let Some(relative_end) = input[note_start..].find('］') else {
            output.push_str(&input[start..]);
            return output;
        };
        let end = note_start + relative_end + '］'.len_utf8();
        let marker = &input[start..end];
        if let Some(replacement) = gaiji_replacement(marker, gaiji) {
            if replacement.chars().count() == 1
                && replacement
                    .chars()
                    .next()
                    .is_some_and(|character| matches!(character, '※' | '《' | '》' | '｜' | '＃'))
            {
                output.push('※');
            }
            output.push_str(&replacement);
        } else {
            output.push_str(marker);
        }
        cursor = end;
    }
    output.push_str(&input[cursor..]);
    output
}

fn gaiji_replacement(
    marker: &str,
    gaiji: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    let normalized = crate::config::normalize_gaiji_key(marker);
    let bare = marker
        .strip_prefix("※［＃")
        .and_then(|value| value.strip_suffix('］'));
    let key = bare.and_then(|value| value.split('、').next());
    gaiji
        .get(marker)
        .or_else(|| gaiji.get(&normalized))
        .or_else(|| bare.and_then(|value| gaiji.get(value)))
        .or_else(|| key.and_then(|value| gaiji.get(value)))
        .or_else(|| {
            key.map(crate::config::normalize_gaiji_key)
                .and_then(|value| gaiji.get(&value))
        })
        .cloned()
        .or_else(|| unicode_gaiji_replacement(marker))
}

fn unicode_gaiji_replacement(note: &str) -> Option<String> {
    let upper = note.to_ascii_uppercase();
    let marker = upper.find("U+")?;
    let start = marker + 2;
    let end = upper[start..]
        .char_indices()
        .find_map(|(offset, character)| (!character.is_ascii_hexdigit()).then_some(start + offset))
        .unwrap_or(upper.len());
    let code = u32::from_str_radix(&upper[start..end], 16).ok()?;
    let mut replacement = char::from_u32(code)?.to_string();
    if upper.get(end..).is_some_and(|tail| tail.starts_with("-U+")) {
        let variation_start = end + 3;
        let variation_end = upper[variation_start..]
            .char_indices()
            .find_map(|(offset, character)| {
                (!character.is_ascii_hexdigit()).then_some(variation_start + offset)
            })
            .unwrap_or(upper.len());
        let variation = u32::from_str_radix(&upper[variation_start..variation_end], 16).ok()?;
        replacement.push(char::from_u32(variation)?);
    }
    Some(replacement)
}

/// Same as [`detect_meta`] with a caller-provided page-break note set.
pub fn detect_meta_with_page_breaks(
    input: &str,
    title_type: TitleType,
    publisher_first: bool,
    page_breaks: &BTreeSet<String>,
) -> BookMeta {
    let (meta_lines, start, first_comment_line) = collect_meta_lines(input, page_breaks);
    let Some(start) = start else {
        return BookMeta::default();
    };
    set_meta_info(
        title_type,
        publisher_first,
        &meta_lines,
        start,
        first_comment_line,
    )
}

/// Collects the leading non-empty lines used for metadata detection,
/// mirroring `AozoraEpub3Converter.getBookInfo`:
///
/// * collection stops at a comment block (50+ dashes), at a page-break
///   chuki, or after 10 collected lines,
/// * blank lines and lines whose text is entirely chuki are skipped but
///   still advance the offset, so gaps are preserved (the `case 2` and
///   `case 1` layouts depend on them).
///
/// Returns (up to 10 line slots, first line index, first comment line index).
fn collect_meta_lines(
    input: &str,
    page_breaks: &BTreeSet<String>,
) -> (Vec<Option<String>>, Option<usize>, Option<usize>) {
    const BUFFER: usize = 10;
    let mut slots = vec![None; BUFFER];
    let mut start = None;
    let mut first_comment_line = None;

    for (line_index, raw_line) in input.lines().enumerate() {
        let line = remove_space(raw_line);
        let no_ruby = remove_ruby(&line);
        if no_ruby.starts_with(COMMENT_MARKER) {
            first_comment_line = Some(line_index);
            break;
        }
        if no_ruby.is_empty() {
            continue;
        }
        match start {
            None => {
                if !chapter_name(&no_ruby, 0, true).is_empty() {
                    start = Some(line_index);
                    slots[0] = Some(line);
                }
            }
            Some(first_line) => {
                if is_page_break_line(&no_ruby, page_breaks) {
                    break;
                }
                let offset = line_index - first_line;
                if offset > BUFFER - 1 {
                    break;
                }
                if !chapter_name(&no_ruby, 0, true).is_empty() {
                    slots[offset] = Some(line);
                }
            }
        }
    }
    (slots, start, first_comment_line)
}

/// True when the line contains a page-break chuki such as ［＃改ページ］.
fn is_page_break_line(line: &str, page_breaks: &BTreeSet<String>) -> bool {
    let mut rest = line;
    let marker_len = "［＃".len();
    let close_len = '］'.len_utf8();
    while let Some(open) = rest.find("［＃") {
        let after = &rest[open + marker_len..];
        let Some(close) = after.find('］') else {
            return false;
        };
        if page_breaks.contains(&after[..close]) {
            return true;
        }
        rest = &after[close + close_len..];
    }
    false
}

/// Port of `BookInfo.setMetaInfo`: assigns title/creator/publisher lines
/// from the leading non-empty lines according to the title type.
///
/// `meta_lines` holds up to 10 slots with `None` for skipped (blank) lines.
fn set_meta_info(
    title_type: TitleType,
    publisher_first: bool,
    meta_lines: &[Option<String>],
    meta_line_start: usize,
    first_comment_line: Option<usize>,
) -> BookMeta {
    let mut meta = BookMeta::default();
    if title_type == TitleType::None {
        return meta;
    }
    // Java stores the original start line before the publisher shift
    meta.meta_line_start = Some(meta_line_start);

    let mut lines_length = meta_lines.iter().take_while(|line| line.is_some()).count();
    let mut arr_index = 0;
    let mut start = meta_line_start;

    // publisher-first: the very first line is the publisher (only when at
    // least two lines follow)
    if publisher_first && lines_length >= 2 {
        meta.publisher = meta_lines[0].clone();
        meta.publisher_line = Some(start);
        start += 1;
        lines_length -= 1;
        arr_index += 1;
    }

    let line_at = |offset: usize| -> Option<&str> {
        meta_lines
            .get(offset + arr_index)
            .and_then(|line| line.as_deref())
    };

    if lines_length > 0 && title_type == TitleType::TitleOnly {
        meta.title_line = Some(start);
        meta.title = line_at(0).map(str::to_owned);
        meta.title_end_line = Some(start);
    } else if lines_length > 0 && title_type == TitleType::TitleAuthorOnly {
        meta.title_line = Some(start);
        meta.title = line_at(0).map(str::to_owned);
        meta.creator_line = Some(start + 1);
        meta.creator = line_at(1).map(str::to_owned);
        meta.title_end_line = Some(start + 1);
    } else {
        match lines_length.min(6) {
            6 => {
                if title_type.title_first() {
                    meta.title_line = Some(start);
                    meta.subtitle_line = Some(start + 2);
                    meta.title = join_two(line_at(0), line_at(2));
                    meta.title_end_line = Some(start + 3);
                    if title_type.has_author() {
                        meta.creator_line = Some(start + 4);
                        meta.creator = line_at(4).map(str::to_owned);
                        meta.title_end_line = Some(start + 5);
                    }
                } else {
                    meta.creator_line = Some(start);
                    meta.creator = line_at(0).map(str::to_owned);
                    meta.title_end_line = Some(start + 1);
                    if title_type.has_title() {
                        meta.title_line = Some(start + 2);
                        meta.subtitle_line = Some(start + 4);
                        meta.title = join_two(line_at(2), line_at(4));
                        meta.title_end_line = Some(start + 5);
                    }
                }
            }
            5 => {
                if title_type.title_first() {
                    meta.title_line = Some(start);
                    meta.subtitle_line = Some(start + 2);
                    meta.title = join_two(line_at(0), line_at(2));
                    meta.title_end_line = Some(start + 2);
                    if title_type.has_author() {
                        meta.creator_line = Some(start + 3);
                        meta.creator = line_at(3).map(str::to_owned);
                        meta.title_end_line = Some(start + 4);
                    }
                } else {
                    meta.creator_line = Some(start);
                    meta.creator = line_at(0).map(str::to_owned);
                    meta.title_end_line = Some(start);
                    if title_type.has_title() {
                        meta.title_line = Some(start + 1);
                        meta.subtitle_line = Some(start + 3);
                        meta.title = join_two(line_at(1), line_at(3));
                    }
                    meta.title_end_line = Some(start + 4);
                }
            }
            4 => {
                if title_type.title_first() {
                    meta.title_line = Some(start);
                    meta.subtitle_line = Some(start + 1);
                    meta.title = join_two(line_at(0), line_at(1));
                    meta.title_end_line = Some(start + 1);
                    if title_type.has_author() {
                        meta.creator_line = Some(start + 2);
                        meta.creator = line_at(2).map(str::to_owned);
                        meta.title_end_line = Some(start + 3);
                    }
                } else {
                    meta.creator_line = Some(start);
                    meta.creator = line_at(0).map(str::to_owned);
                    meta.title_end_line = Some(start + 1);
                    if title_type.has_title() {
                        meta.title_line = Some(start + 2);
                        meta.subtitle_line = Some(start + 3);
                        meta.title = join_two(line_at(2), line_at(3));
                        meta.title_end_line = Some(start + 3);
                    }
                }
            }
            3 => {
                if title_type.title_first() {
                    meta.title_line = Some(start);
                    meta.subtitle_line = Some(start + 1);
                    meta.title = join_two(line_at(0), line_at(1));
                    meta.title_end_line = Some(start + 1);
                    if title_type.has_author() {
                        // 表題+著者+翻訳者 (title, author, translator)
                        let second = line_at(1).unwrap_or_default();
                        let third = line_at(2).unwrap_or_default();
                        if title_type != TitleType::SubtitleAuthor
                            && !second.starts_with('―')
                            && (third.ends_with("訳")
                                || third.ends_with("編纂")
                                || third.ends_with("校訂"))
                        {
                            meta.title = line_at(0).map(str::to_owned);
                            meta.subtitle_line = None;
                            meta.creator_line = Some(start + 1);
                            meta.creator = Some(second.to_owned());
                        } else {
                            meta.creator_line = Some(start + 2);
                            meta.creator = line_at(2).map(str::to_owned);
                        }
                        meta.title_end_line = Some(start + 2);
                    }
                } else {
                    meta.creator_line = Some(start);
                    meta.creator = line_at(0).map(str::to_owned);
                    meta.title_end_line = Some(start);
                    if title_type.has_title() {
                        meta.title_line = Some(start + 1);
                        meta.subtitle_line = Some(start + 2);
                        meta.title = join_two(line_at(1), line_at(2));
                        meta.title_end_line = Some(start + 2);
                    }
                }
            }
            2 => {
                if title_type.title_first() {
                    meta.title_line = Some(start);
                    meta.title = line_at(0).map(str::to_owned);
                    if title_type.has_author() {
                        // title+subtitle, blank, author — only when a
                        // comment follows within 6 lines and the layout fits
                        let comment = first_comment_line
                            .filter(|line| *line > 0 && *line <= 6)
                            .is_some();
                        let third = line_at(3);
                        let fourth = line_at(4);
                        if comment
                            && third.is_some_and(|line| !line.is_empty())
                            && fourth.is_none_or(str::is_empty)
                        {
                            meta.subtitle_line = Some(start + 1);
                            meta.title = join_two(line_at(0), line_at(1));
                            meta.creator_line = Some(start + 3);
                            meta.creator = line_at(2).map(str::to_owned);
                            meta.title_end_line = Some(start + 3);
                        } else {
                            meta.creator_line = Some(start + 1);
                            meta.creator = line_at(1).map(str::to_owned);
                            meta.title_end_line = Some(start + 1);
                        }
                    }
                } else {
                    meta.creator_line = Some(start);
                    meta.creator = line_at(0).map(str::to_owned);
                    if title_type.has_title() {
                        meta.title_line = Some(start + 1);
                        meta.title = line_at(1).map(str::to_owned);
                    }
                    meta.title_end_line = Some(start + 1);
                }
            }
            1 => {
                if title_type.title_first() {
                    meta.title_line = Some(start);
                    meta.title = line_at(0).map(str::to_owned);
                    meta.title_end_line = Some(start);
                    if title_type.has_author() {
                        // title, blank, creator — the classic 3-line layout
                        let third = line_at(2);
                        let fourth = line_at(3);
                        if third.is_some_and(|line| !line.is_empty())
                            && fourth.is_none_or(str::is_empty)
                        {
                            meta.creator_line = Some(start + 2);
                            meta.creator = line_at(2).map(str::to_owned);
                            meta.title_end_line = Some(start + 2);
                        }
                    }
                } else {
                    meta.creator_line = Some(start);
                    meta.creator = line_at(0).map(str::to_owned);
                    meta.title_end_line = Some(start);
                    if title_type.has_title() {
                        let third = line_at(2);
                        let fourth = line_at(3);
                        if third.is_some_and(|line| !line.is_empty())
                            && fourth.is_none_or(str::is_empty)
                        {
                            meta.title_line = Some(start + 2);
                            meta.title = line_at(2).map(str::to_owned);
                            meta.title_end_line = Some(start + 2);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Post-processing from BookInfo.setMetaInfo: drop bracket-y creators,
    // then clean title/creator text.
    if meta
        .creator
        .as_deref()
        .is_some_and(|creator| creator.starts_with('―') || creator.starts_with('【'))
    {
        meta.creator = None;
    }
    if let Some(title) = meta.title.take() {
        meta.title = normalize_text(&title, false);
    }
    if let Some(creator) = meta.creator.take() {
        meta.creator = normalize_text(&creator, true);
    }
    meta
}

fn join_two(first: Option<&str>, second: Option<&str>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first} {second}")),
        (Some(first), None) => Some(first.to_owned()),
        (None, Some(second)) => Some(second.to_owned()),
        (None, None) => None,
    }
}

fn normalize_text(text: &str, reduce: bool) -> Option<String> {
    let text = remove_metadata_image_notes(text);
    let cleaned = chapter_name(&remove_ruby(&text), 0, reduce);
    (!cleaned.is_empty()).then_some(cleaned)
}

fn remove_metadata_image_notes(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    while cursor < input.len() {
        let marker = input[cursor..].find("［＃").map(|offset| cursor + offset);
        let Some(marker_start) = marker else {
            output.push_str(&input[cursor..]);
            break;
        };
        output.push_str(&input[cursor..marker_start]);
        let note_start = marker_start + "［＃".len();
        let Some(close_offset) = input[note_start..].find('］') else {
            output.push_str(&input[marker_start..]);
            break;
        };
        let note_end = note_start + close_offset + '］'.len_utf8();
        let note = &input[note_start..note_start + close_offset];
        let is_image = note.contains('（') && note.contains('.') && note.contains('）');
        if is_image {
            if output.ends_with('※') {
                output.pop();
            }
        } else {
            output.push_str(&input[marker_start..note_end]);
        }
        cursor = note_end;
    }
    output
}

/// Parses `[creator] title` (and fallback forms) from a file name,
/// matching `BookInfo.getFileTitleCreator`.
///
/// Returns `(title, creator)`, either of which may be `None`.
pub fn file_title_creator(file_name: &str) -> (Option<String>, Option<String>) {
    // strip one or two trailing ASCII extensions (".txt", ".kepub.epub")
    let mut no_ext = file_name;
    for _ in 0..2 {
        let Some(stripped) = strip_ascii_extension(no_ext) else {
            break;
        };
        no_ext = stripped;
    }
    // normalize full-width parentheses for the cleanup rules
    let normalized = no_ext.replace('（', "(").replace('）', ")");
    // drop 青空文庫 and 校正/軽量/表紙/挿絵/補正/修正/ルビ notes
    let cleaned = remove_cleanup_notes(&normalized);

    if let Some((creator, title)) = bracket_creator_title(&cleaned) {
        return (trimmed(&title), trimmed(&creator));
    }
    if let Some(title) = parenthesized_title(&cleaned) {
        return (trimmed(&title), None);
    }
    (trimmed(&cleaned), None)
}

fn strip_ascii_extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    let extension = &name[dot + 1..];
    (!extension.is_empty() && extension.chars().all(|c| c.is_ascii_alphanumeric()))
        .then_some(&name[..dot])
}

fn remove_cleanup_notes(text: &str) -> String {
    let text = remove_keyword_parenthetical(text, &["青空"], true);
    remove_keyword_parenthetical(
        &text,
        &[
            "校正", "軽量", "表紙", "挿絵", "補正", "修正", "ルビ", "Rev", "rev",
        ],
        false,
    )
}

/// Removes `(...)` parentheticals whose inner content contains a keyword.
/// When `prefix_only` is set the keyword must appear right after the `(`.
fn remove_keyword_parenthetical(text: &str, keywords: &[&str], prefix_only: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find(')') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let inner = &after[..close];
        let matches = if prefix_only {
            inner.starts_with(keywords[0])
        } else {
            keywords.iter().any(|keyword| inner.contains(keyword))
        };
        if matches {
            rest = &after[close + 1..];
        } else {
            out.push('(');
            out.push_str(inner);
            out.push(')');
            rest = &after[close + 1..];
        }
    }
    out.push_str(rest);
    out
}

/// `[creator] title` extraction. The bracket class matches the Java pattern
/// `[\[|［](.+?)[\]|］][ |　]*(.*)[ |　]*$` (including the odd `|` member).
fn bracket_creator_title(text: &str) -> Option<(String, String)> {
    let open = text
        .char_indices()
        .find_map(|(index, character)| matches!(character, '[' | '|' | '［').then_some(index))?;
    let after_open = &text[open + text[open..].chars().next()?.len_utf8()..];
    let (close_offset, close_len) = after_open.char_indices().find_map(|(index, character)| {
        matches!(character, ']' | '|' | '］').then_some((index, character.len_utf8()))
    })?;
    let creator = after_open[..close_offset].to_owned();
    if creator.is_empty() {
        return None;
    }
    let title = after_open[close_offset + close_len..]
        .trim_matches([' ', '　'])
        .to_owned();
    Some((creator, title))
}

/// `title (…)` extraction: everything before the first `(`, trimmed.
fn parenthesized_title(text: &str) -> Option<String> {
    let open = text.find(['(', '（'])?;
    Some(text[..open].to_owned())
}

fn trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

// ---------------------------------------------------------------------------
// CharUtils ports
// ---------------------------------------------------------------------------

/// Trims leading/trailing ASCII and full-width spaces.
fn remove_space(text: &str) -> String {
    text.trim_matches([' ', '　']).to_owned()
}

/// Removes ruby markup `｜漢字《かんじ》` (respecting `※` escapes), matching
/// `CharUtils.removeRuby`.
fn remove_ruby(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(text.len());
    let mut in_ruby = false;
    for (index, character) in chars.iter().enumerate() {
        if in_ruby {
            if *character == '》' && !is_escaped(&chars, index) {
                in_ruby = false;
            }
        } else {
            match character {
                '｜' => {
                    if is_escaped(&chars, index) {
                        out.push(*character);
                    }
                }
                '《' => {
                    if is_escaped(&chars, index) {
                        out.push(*character);
                    } else {
                        in_ruby = true;
                    }
                }
                _ => out.push(*character),
            }
        }
    }
    out
}

/// True when the character is escaped by an odd run of `※` immediately
/// before it (matching `CharUtils.isEscapedChar`).
fn is_escaped(chars: &[char], index: usize) -> bool {
    let mut escaped = false;
    for character in chars[..index].iter().rev() {
        if *character == '※' {
            escaped = !escaped;
        } else {
            break;
        }
    }
    escaped
}

/// Extracts a chapter/title name, matching `CharUtils.getChapterName`:
/// chuki removal, `※` unescaping, whitespace trimming, optional symbol
/// collapsing, and `<img>`/`<a>` tag removal.
fn chapter_name(line: &str, max_length: usize, reduce: bool) -> String {
    let mut name = remove_chuki(line);
    name = unescape_marks(&name);
    name = name.replace('\t', " ");
    name = remove_space(&name);
    if reduce {
        name = collapse_symbols(&name);
    }
    name = remove_image_anchor_tags(&name);
    if max_length == 0 {
        return name;
    }
    if name.chars().count() > max_length {
        let truncated = name.chars().take(max_length).collect::<String>();
        format!("{truncated}...")
    } else {
        name
    }
}

/// Removes ［＃…］ chuki notes (up to the first ］).
fn remove_chuki(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(character) = chars.next() {
        if character == '［' && chars.clone().next() == Some('＃') {
            chars.next();
            for next in chars.by_ref() {
                if next == '］' {
                    break;
                }
            }
        } else {
            out.push(character);
        }
    }
    out
}

/// Drops a `※` immediately before one of `※《》［］〔〕｜`, matching the
/// Java `※([※《》［］〔〕｜])` replacement (left to right, non-overlapping).
fn unescape_marks(line: &str) -> String {
    let chars = line.chars().collect::<Vec<_>>();
    let mut out = String::with_capacity(line.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '※' && index + 1 < chars.len() && special_next(&chars, index + 1) {
            out.push(chars[index + 1]);
            index += 2;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    out
}

fn special_next(chars: &[char], index: usize) -> bool {
    matches!(
        chars[index],
        '※' | '《' | '》' | '［' | '］' | '〔' | '〕' | '｜'
    )
}

/// Collapses runs of `= ＝ - ― ─` into a single character, matching the
/// Java `([=＝\-―─])+ -> $1` rule (keeps the first character of the run).
fn collapse_symbols(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut previous_symbol = false;
    for character in line.chars() {
        let symbol = matches!(character, '=' | '＝' | '-' | '―' | '─');
        if symbol && previous_symbol {
            continue;
        }
        out.push(character);
        previous_symbol = symbol;
    }
    out
}

/// Removes `<img …>` / `<a …>` tags (open and close), matching
/// `CharUtils.chapterTagOpenPattern` / `chapterTagClosePattern`.
fn remove_image_anchor_tags(line: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let mut out = String::with_capacity(line.len());
    let mut index = 0;
    while index < line.len() {
        if line.as_bytes()[index] == b'<' {
            let Some(relative_end) = line[index..].find('>') else {
                out.push_str(&line[index..]);
                break;
            };
            let end = index + relative_end + 1;
            let tag = &lower[index..end];
            let inner = tag[1..tag.len() - 1].trim();
            let is_image_anchor = inner.starts_with("img")
                || inner.starts_with("/img")
                || inner.starts_with("a ")
                || inner.starts_with("a/")
                || inner.starts_with("a>")
                || inner.starts_with("/a");
            if is_image_anchor {
                index = end;
                continue;
            }
        }
        let character = line[index..].chars().next().unwrap();
        out.push(character);
        index += character.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{BookMeta, TitleType, detect_meta, detect_meta_with_gaiji, file_title_creator};
    use std::collections::BTreeMap;

    fn meta(input: &str, title_type: TitleType) -> BookMeta {
        detect_meta(input, title_type, false)
    }

    #[test]
    fn resolves_unicode_gaiji_notes_before_metadata_detection() {
        let gaiji = BTreeMap::new();
        let book = detect_meta_with_gaiji(
            "外字※［＃サッカーボール、U+26BD、ページ数-行数］\n著者\n\n本文",
            TitleType::TitleAuthor,
            false,
            &gaiji,
        );
        assert_eq!(book.title.as_deref(), Some("外字⚽"));
        assert_eq!(book.creator.as_deref(), Some("著者"));
    }
    #[test]
    fn resolves_gaiji_notes_with_location_suffixes() {
        let mut gaiji = BTreeMap::new();
        gaiji.insert("二の字点".to_owned(), "〻".to_owned());
        let book = detect_meta_with_gaiji(
            "表題※［＃二の字点、1-2-22、ページ数-行数］\n著者\n\n本文",
            TitleType::TitleAuthor,
            false,
            &gaiji,
        );
        assert_eq!(book.title.as_deref(), Some("表題〻"));
    }

    #[test]
    fn title_type_ordering_and_flags() {
        let flags = TitleType::ALL
            .iter()
            .map(|t| (t.title_first(), t.has_title(), t.has_author()))
            .collect::<Vec<_>>();
        assert_eq!(
            flags,
            vec![
                (true, true, true),    // TitleAuthor
                (false, true, true),   // AuthorTitle
                (true, true, true),    // SubtitleAuthor
                (true, true, false),   // TitleOnly
                (true, true, true),    // TitleAuthorOnly
                (false, false, false)  // None
            ]
        );
        assert_eq!(TitleType::from_index(0), Some(TitleType::TitleAuthor));
        assert_eq!(TitleType::from_index(5), Some(TitleType::None));
        assert_eq!(TitleType::from_index(6), None);
    }

    #[test]
    fn detects_title_and_creator_on_separate_lines() {
        // classic layout: title, author, blank line
        let book = meta("表題\n著者名\n\n本文…", TitleType::TitleAuthor);
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator.as_deref(), Some("著者名"));
        assert_eq!(book.title_line, Some(0));
        assert_eq!(book.creator_line, Some(1));
        assert_eq!(book.meta_line_start, Some(0));
        assert_eq!(book.title_end_line, Some(1));
    }

    #[test]
    fn skips_blank_lines_but_keeps_offsets_for_gap_layouts() {
        // title, blank, creator, blank — case 1 layout at offsets 0 and 2
        let book = meta("表題\n\n著者名\n\n本文", TitleType::TitleAuthor);
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator.as_deref(), Some("著者名"));
        assert_eq!(book.title_line, Some(0));
        assert_eq!(book.creator_line, Some(2));
        assert_eq!(book.title_end_line, Some(2));

        // title, subtitle, blank, author, comment — case 2 with the
        // comment-triggered subtitle branch. Java reads the blank slot for
        // the creator here (a latent BookInfo quirk), so the creator stays
        // unset even though creatorLine points at the author line.
        let commented = format!("表題\n副題\n\n著者名\n{}\n本文", "-".repeat(50));
        let book = meta(&commented, TitleType::TitleAuthor);
        assert_eq!(book.title.as_deref(), Some("表題 副題"));
        assert_eq!(book.subtitle_line, Some(1));
        assert_eq!(book.creator, None);
        assert_eq!(book.creator_line, Some(3));
        assert_eq!(book.title_end_line, Some(3));
    }

    #[test]
    fn stops_metadata_collection_at_comment_blocks_and_page_breaks() {
        let commented = format!("表題\n{}\n著者名\n本文", "-".repeat(50));
        let book = meta(&commented, TitleType::TitleAuthor);
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator, None);

        let book = meta("表題\n［＃改ページ］\n著者名\n本文", TitleType::TitleAuthor);
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator, None);
    }

    #[test]
    fn treats_symbol_only_lines_as_blank_for_metadata() {
        let book = meta(
            "［＃改ページ］\n表題\n著者名\n\n本文",
            TitleType::TitleAuthor,
        );
        assert_eq!(book.meta_line_start, Some(1));
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator.as_deref(), Some("著者名"));
    }

    #[test]
    fn author_title_type_swaps_lines() {
        let book = meta("著者名\n表題\n\n本文", TitleType::AuthorTitle);
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator.as_deref(), Some("著者名"));
        assert_eq!(book.title_line, Some(1));
        assert_eq!(book.creator_line, Some(0));
    }

    #[test]
    fn title_only_and_title_author_only_types() {
        let book = meta("表題\n本文", TitleType::TitleOnly);
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator, None);

        let book = meta("表題\n著者名\n本文", TitleType::TitleAuthorOnly);
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator.as_deref(), Some("著者名"));
        assert_eq!(book.title_end_line, Some(1));
    }

    #[test]
    fn subtitle_author_keeps_title_subtitle_creator_lines() {
        let input = "表題\n副題\n著者名\n本文";
        let book = meta(input, TitleType::SubtitleAuthor);
        assert_eq!(book.title.as_deref(), Some("表題 副題"));
        assert_eq!(book.creator.as_deref(), Some("著者名"));
        assert_eq!(book.subtitle_line, Some(1));
        assert_eq!(book.creator_line, Some(2));
    }

    #[test]
    fn detects_translator_layout_in_three_lines() {
        // 表題 + 著者 + 翻訳者 when the third line ends with 訳
        let book = meta("表題\n夏目漱石\n金原瑞人訳\n\n本文", TitleType::TitleAuthor);
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator.as_deref(), Some("夏目漱石"));
        assert_eq!(book.subtitle_line, None);
        assert_eq!(book.creator_line, Some(1));
    }

    #[test]
    fn removes_ruby_and_chuki_from_title_text() {
        let book = meta(
            "｜表題《ひょうだい》\n著者名\n\n本文",
            TitleType::TitleAuthor,
        );
        assert_eq!(book.title.as_deref(), Some("表題"));
        // ※-escaped ruby markers survive
        let book = meta("※《表題》\n著者名\n\n本文", TitleType::TitleAuthor);
        assert_eq!(book.title.as_deref(), Some("《表題》"));
    }

    #[test]
    fn unescapes_adjacent_escaped_marks_without_overlapping() {
        let book = meta(
            "｜ルビ※［＃米印］《るび》※［＃米印］※［＃始め二重山括弧］※［＃終わり二重山括弧］\n\
             著者\n\
             \n本文",
            TitleType::TitleAuthor,
        );
        assert_eq!(book.title.as_deref(), Some("ルビ※※"));
    }

    #[test]
    fn removes_image_notes_from_title_and_creator() {
        let book = meta(
            "縦中横※［＃図形（fig.png、横19×縦15）入る］AAA\n\
             著者※［＃図形（author.png）入る］\n\
             \n本文",
            TitleType::TitleAuthor,
        );
        assert_eq!(book.title.as_deref(), Some("縦中横AAA"));
        assert_eq!(book.creator.as_deref(), Some("著者"));
    }

    #[test]
    fn publisher_first_consumes_the_first_line() {
        let book = detect_meta("刊行社\n表題\n著者名\n\n本文", TitleType::TitleAuthor, true);
        assert_eq!(book.publisher.as_deref(), Some("刊行社"));
        assert_eq!(book.publisher_line, Some(0));
        assert_eq!(book.title.as_deref(), Some("表題"));
        assert_eq!(book.creator.as_deref(), Some("著者名"));
        assert_eq!(book.meta_line_start, Some(0));
        assert_eq!(book.title_end_line, Some(2));

        let book = detect_meta("刊行社\n表題\n著者名\n本文", TitleType::TitleAuthor, false);
        assert_eq!(book.publisher, None);
        assert_eq!(book.title.as_deref(), Some("刊行社 表題"));
    }

    #[test]
    fn none_title_type_returns_no_text_metadata() {
        let book = meta("表題\n著者名\n本文", TitleType::None);
        assert_eq!(book, BookMeta::default());
    }

    #[test]
    fn parses_creator_bracket_titles_from_file_names() {
        assert_eq!(
            file_title_creator("［夏目漱石］吾輩は猫である.txt"),
            (
                Some("吾輩は猫である".to_owned()),
                Some("夏目漱石".to_owned())
            )
        );
        assert_eq!(
            file_title_creator("[creator] title.txt"),
            (Some("title".to_owned()), Some("creator".to_owned()))
        );
        assert_eq!(
            file_title_creator("[creator] 前後空白.txt"),
            (Some("前後空白".to_owned()), Some("creator".to_owned()))
        );
        // trailing spaces around the title are trimmed
        assert_eq!(
            file_title_creator("[creator] title  .txt"),
            (Some("title".to_owned()), Some("creator".to_owned()))
        );
    }

    #[test]
    fn strips_cleanup_notes_and_parens_from_file_names() {
        assert_eq!(
            file_title_creator("吾輩は猫である（青空文庫）.txt"),
            (Some("吾輩は猫である".to_owned()), None)
        );
        assert_eq!(
            file_title_creator("吾輩は猫である(校正済).txt"),
            (Some("吾輩は猫である".to_owned()), None)
        );
        assert_eq!(
            file_title_creator("タイトル（改訂版）.txt"),
            (Some("タイトル".to_owned()), None)
        );
        assert_eq!(
            file_title_creator("plain_name.txt"),
            (Some("plain_name".to_owned()), None)
        );
        // two extensions are stripped
        assert_eq!(
            file_title_creator("name.kepub.epub"),
            (Some("name".to_owned()), None)
        );
        // a trailing extension of letters/numbers is always stripped
        assert_eq!(
            file_title_creator("タイトル01.txt"),
            (Some("タイトル01".to_owned()), None)
        );
    }
}
