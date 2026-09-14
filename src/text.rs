use std::fmt;

use crate::config::AozoraConfig;
use encoding_rs::{Encoding, SHIFT_JIS, UTF_8};
#[path = "text_inline.rs"]
mod inline;

use inline::convert_inline;
use inline::convert_inline_at;
use inline::convert_inline_span;
use inline::convert_inline_with_yoko;
use inline::phase1_markup_len;
use inline::phase1_text_len;
pub fn inline_to_xhtml(input: &str, config: &AozoraConfig) -> String {
    convert_inline(input, config)
}
pub use inline::{
    apply_alt_upright, collect_image_alts, escape_html, image_reference_occurrences,
    image_references, tcy_label,
};

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
    Ok(render_lines(lines.iter().map(String::as_str), &[], &[], config).0)
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
    /// Section (xhtml file) index the chapter points at.
    pub section_index: usize,
    /// Line index inside the section where the chapter anchor is emitted.
    pub line_index: usize,
    /// Normalized navigation label.
    pub label: String,
    /// 1-based heading level (1 for page-break chapters).
    pub level: u8,
    /// Java `ChapterLineInfo.pageBreakChapter`: the chapter sits on the
    /// first content line after a page break, so the TOC links to the
    /// section file without a `#kobo.N.M` fragment.
    pub page_break_chapter: bool,
    /// `kobo.N.M` id emitted on the chapter line; `None` until the section
    /// is rendered. Only non-`page_break_chapter` records get a TOC
    /// fragment.
    pub anchor: Option<String>,
    /// Java `ChapterLineInfo.lineNum` 相当の通し行番号
    /// (`ChapterExclude` の前後判定に使う)。
    pub source_line: usize,
    /// Java `ChapterLineInfo.type` 相当 (`ChapterExclude` の判定に使う)。
    pub kind: ChapterKind,
    /// Java `ChapterLineInfo.emptyNext`: 直前の行が空行だったか。
    pub empty_next: bool,
}

/// Java `ChapterLineInfo.TYPE_*` 相当。`is_pattern` が真の種別だけが
/// `ChapterExclude` の対象になる。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChapterKind {
    Title,
    PageBreak,
    ChukiH1,
    ChukiH2,
    ChukiH3,
    ChapterName,
    ChapterNum,
    Pattern,
}

impl ChapterKind {
    /// Java `ChapterLineInfo.isPattern`: 章名・数字・パターンでマッチした行。
    fn is_pattern(self) -> bool {
        matches!(self, Self::ChapterName | Self::ChapterNum | Self::Pattern)
    }
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

/// Heading level for a ［＃...］ note name, mirroring Java's
/// `chapterChukiMap` + `ChapterLineInfo.getLevel`: 見出し/大見出し → 1,
/// 中見出し → 2, 小見出し → 3. Only INI-enabled types match; `ここから`
/// block-open variants share their level, `同行` variants additionally need
/// `SameLineChapter`.
fn heading_note_level(note: &str, config: &AozoraConfig) -> Option<u8> {
    let (base, level) = match note {
        "見出し" | "ここから見出し" => (0, 1),
        "大見出し" | "ここから大見出し" => (1, 1),
        "中見出し" | "ここから中見出し" => (2, 2),
        "小見出し" | "ここから小見出し" => (3, 3),
        "同行見出し" => (4, 1),
        "同行大見出し" => (5, 1),
        "同行中見出し" => (6, 2),
        "同行小見出し" => (7, 3),
        _ => return None,
    };
    let enabled = match base {
        0 => config.chapter_h,
        1 => config.chapter_h1,
        2 => config.chapter_h2,
        3 => config.chapter_h3,
        _ => {
            config.same_line_chapter
                && match base {
                    4 => config.chapter_h,
                    5 => config.chapter_h1,
                    6 => config.chapter_h2,
                    _ => config.chapter_h3,
                }
        }
    };
    enabled.then_some(level)
}

/// All ［＃...］ note names in a line, in order, each paired with the byte
/// offset just past its closing ］. Chapter detection (like Java's pre-read)
/// scans every line for heading notes, not just section heads.
fn line_note_names(line: &str) -> Vec<(String, usize)> {
    let mut notes = Vec::new();
    let mut search_from = 0;
    while let Some(relative) = line[search_from..].find("［＃") {
        let note_start = search_from + relative + "［＃".len();
        let Some(relative_close) = line[note_start..].find('］') else {
            break;
        };
        let after_end = note_start + relative_close + '］'.len_utf8();
        notes.push((
            line[note_start..note_start + relative_close].to_owned(),
            after_end,
        ));
        search_from = after_end;
    }
    notes
}

/// Splits the input into body sections and detects navigation chapters the
/// way the reference converter's pre-read does: chapters come from heading
/// notes anywhere in a line (type-gated by the `ChapterH*` INI keys) and,
/// with `ChapterSection`, from the first non-symbol line of each section
/// after a page break. `initial_add_section_chapter` mirrors the pre-read
/// state after the title line: `false` when the metadata block already
/// consumed the first-chapter slot (so `input` should be the meta-stripped
/// body in that case).
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
    // (line index in `current`, index into `chapters`) for chapter lines in
    // the section being accumulated.
    let mut chapter_lines: Vec<(usize, usize)> = Vec::new();
    // Java pre-read: a heading note with nothing after it takes its name
    // from the next visible line only. Holds (level, page_break_chapter).
    let mut pending_heading: Option<(u8, bool)> = None;
    // Java `addNextChapterName`: 章名に繋げる次の行番号。
    let mut add_next_chapter_name: Option<usize> = None;
    // Java `lastEmptyLine`: 直近の空行番号 (`emptyNext` 判定に使う)。
    let mut last_empty_line: Option<usize> = None;

    for (line_index, line) in visible_lines(input, config).iter().enumerate() {
        let line_start_section = section_index;

        if crate::metadata::remove_ruby(line)
            .trim_matches([' ', '　'])
            .is_empty()
        {
            last_empty_line = Some(line_index);
        }

        // 見出しの次の行を章名に繋げる (Java `addNextChapterName`)
        if add_next_chapter_name == Some(line_index)
            && !chapter_lines
                .iter()
                .any(|(index, _)| *index == current.len())
            && let Some(record) = chapters.last_mut()
        {
            let name = chapter_name(line, config);
            if !name.is_empty() {
                // Java ChapterLineInfo.joinChapterName: 全角空白で連結する
                record.label.push('　');
                record.label.push_str(&name);
            }
            add_next_chapter_name = None;
        }

        // Resolve a deferred heading name before anything else on this line;
        // an empty name drops the pending chapter (Java clears the slot).
        if let Some((level, pbc)) = pending_heading.take() {
            let name = chapter_name(line, config);
            if !name.is_empty() {
                chapter_lines.push((current.len(), chapters.len()));
                chapters.push(ChapterRecord {
                    section_index: line_start_section,
                    line_index: current.len(),
                    label: name,
                    level,
                    page_break_chapter: pbc,
                    anchor: None,
                    source_line: line_index,
                    kind: chapter_kind_for_level(level),
                    empty_next: last_empty_line == Some(line_index.wrapping_sub(1)),
                });
                add_section_chapter = false;
            }
        }

        let has_page_break = find_page_break_note(line, config).is_some();
        // Java splits on page-break notes unconditionally; the `PageBreak`
        // INI key only enables size-based splitting (`force_page_break`).
        if has_page_break {
            add_section_chapter = true;
        }

        // Heading notes register chapters wherever they appear in the line
        // (Java scans every line, not just section heads).
        for (note, after_end) in line_note_names(line) {
            let Some(level) = heading_note_level(&note, config) else {
                continue;
            };
            // A heading chapter replaces the page-break chapter of the same
            // section, exactly like the reference pre-read.
            let pbc = add_section_chapter;
            add_section_chapter = false;
            if after_end >= line.len() {
                pending_heading = Some((level, pbc));
            } else {
                let name = chapter_name(&line[after_end..], config);
                if !name.is_empty() && !symbols_only(&name) {
                    chapter_lines.push((current.len(), chapters.len()));
                    chapters.push(ChapterRecord {
                        section_index: line_start_section,
                        line_index: current.len(),
                        label: name,
                        level,
                        page_break_chapter: pbc,
                        anchor: None,
                        source_line: line_index,
                        kind: chapter_kind_for_level(level),
                        empty_next: last_empty_line == Some(line_index.wrapping_sub(1)),
                    });
                }
            }
        }

        // Java `getBookInfo`: 見出し行パターン抽出。すでに章がある行は対象外。
        if auto_chapter_enabled(config)
            && !chapter_lines
                .iter()
                .any(|(index, _)| *index == current.len())
        {
            let plain = chapter_plain_line(line, config);
            for level in auto_chapter_levels(&plain, config) {
                chapter_lines.push((current.len(), chapters.len()));
                chapters.push(ChapterRecord {
                    section_index: line_start_section,
                    line_index: current.len(),
                    label: chapter_name(line, config),
                    level,
                    page_break_chapter: add_section_chapter,
                    anchor: None,
                    source_line: line_index,
                    kind: if level <= 2 {
                        ChapterKind::ChapterName
                    } else {
                        ChapterKind::ChapterNum
                    },
                    empty_next: last_empty_line == Some(line_index.wrapping_sub(1)),
                });
                if config.chapter_use_next_line {
                    add_next_chapter_name = Some(line_index + 1);
                }
                add_section_chapter = false;
            }
        }

        // `ChapterSection`: the first non-symbol line after a page break is a
        // level-1 chapter unless a heading note already claimed the slot.
        if config.chapter_section && add_section_chapter {
            let name = chapter_name(line, config);
            if !name.is_empty() && !symbols_only(&name) && !is_colophon_line(line) {
                chapter_lines.push((current.len(), chapters.len()));
                chapters.push(ChapterRecord {
                    section_index: line_start_section,
                    line_index: current.len(),
                    label: name,
                    level: 1,
                    page_break_chapter: true,
                    anchor: None,
                    source_line: line_index,
                    kind: ChapterKind::PageBreak,
                    empty_next: last_empty_line == Some(line_index.wrapping_sub(1)),
                });
                add_section_chapter = false;
            } else if is_colophon_line(line) {
                add_section_chapter = false;
            }
        }

        let mut remainder = line.as_str();
        // Java: chukiFlagNoBr (chuki_tag.txt 4列目=1) のブロック注記を含む行は
        // 行全体を <p> で括らない (printLineBuffer noBr)。
        let line_has_block_note = contains_block_note(line, config);
        // 章行は改ページでセクションを flush すると行番号が変わるため、
        // flush したときに付け替える (Java はバッファ単位で扱うので不要)。
        let chapter_line_before_flush = chapter_lines.last().copied();
        loop {
            let Some((offset, end, note)) = find_page_break_note(remainder, config) else {
                append_section_line(
                    remainder,
                    &mut sections,
                    &mut current,
                    &mut no_br,
                    &mut page_marker,
                    &mut section_index,
                    &mut chapter_lines,
                    &mut chapters,
                    config,
                    line_has_block_note,
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
                    &mut chapter_lines,
                    &mut chapters,
                    config,
                    true,
                );
            }
            trim_trailing_empty_lines(&mut current);
            no_br.truncate(current.len());
            if !current.is_empty() {
                push_rendered_section(
                    &mut sections,
                    &mut current,
                    &mut no_br,
                    &mut chapter_lines,
                    &mut chapters,
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
            // Java: ページ左下/ページの左下 のタグは改ページ後の行に出力される
            // (タグは行末で閉じる)。注記自体は行末まで本文を包むため、残りを
            // この行に取り込んで改ページ処理を打ち切る。
            if config.block_inline_tags.contains_key(&note) {
                // 改ページで前のセクションを出力した場合、この行は新しい
                // セクションの先頭になる。章行の対応を付け替える。
                if let Some((recorded, record)) = chapter_line_before_flush
                    && chapter_lines.last().copied() == Some((recorded, record))
                    && recorded != current.len()
                {
                    *chapter_lines.last_mut().unwrap() = (current.len(), record);
                }
                current.push(format!("［＃{note}］{}", &remainder[end..]));
                no_br.push(true);
                break;
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
            &mut chapter_lines,
            &mut chapters,
            page_marker,
            config,
        );
    }
    // Java `BookInfo.excludeTocChapter`: 目次ページの自動抽出見出しを除外する。
    if config.chapter_exclude {
        exclude_toc_chapters(&mut chapters);
    }
    Ok((sections, chapters))
}

/// Java `BookInfo.excludeTocChapter`: 前後 2 行に自動抽出見出しが並ぶ行を
/// 目次から除外する (間は空行のみ許可)。
fn exclude_toc_chapters(chapters: &mut Vec<ChapterRecord>) {
    let is_pattern = |line: usize| {
        chapters
            .iter()
            .find(|record| record.source_line == line)
            .is_some_and(|record| record.kind.is_pattern())
    };
    let mut first = std::collections::BTreeSet::new();
    for record in chapters.iter() {
        if !record.kind.is_pattern() {
            continue;
        }
        let line = record.source_line;
        let previous = line.checked_sub(1).is_some_and(is_pattern)
            || (record.empty_next && line.checked_sub(2).is_some_and(is_pattern));
        let next = is_pattern(line + 1) || is_pattern(line + 2);
        if previous && next {
            first.insert(line);
        }
    }
    let mut second = std::collections::BTreeSet::new();
    for record in chapters.iter() {
        let line = record.source_line;
        if first.contains(&line) || !record.kind.is_pattern() {
            continue;
        }
        let adjacent = line
            .checked_sub(1)
            .is_some_and(|value| first.contains(&value))
            || (record.empty_next
                && line
                    .checked_sub(2)
                    .is_some_and(|value| first.contains(&value)))
            || first.contains(&(line + 1))
            || first.contains(&(line + 2));
        if adjacent {
            second.insert(line);
        }
    }
    chapters.retain(|record| {
        !first.contains(&record.source_line) && !second.contains(&record.source_line)
    });
}

/// Java: chukiFlagNoBr (chuki_tag.txt 4列目=1) のブロック注記を含むか。
/// 含む行は printLineBuffer の noBr 相当で <p> ラップしない。
fn contains_block_note(line: &str, config: &AozoraConfig) -> bool {
    for (start, _) in line.match_indices("［＃") {
        let note_start = start + "［＃".len();
        let Some(close_offset) = line[note_start..].find('］') else {
            continue;
        };
        let note = &line[note_start..note_start + close_offset];
        if config.block_open_tags.contains_key(note)
            || config.block_close_tags.contains_key(note)
            || config.block_inline_tags.contains_key(note)
            || config.block_single_tags.contains_key(note)
            || generated_indent_block(note).is_some()
        {
            return true;
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn push_rendered_section(
    sections: &mut Vec<String>,
    current: &mut Vec<String>,
    no_br: &mut Vec<bool>,
    chapter_lines: &mut Vec<(usize, usize)>,
    chapters: &mut [ChapterRecord],
    page_marker: Option<&'static str>,
    config: &AozoraConfig,
) {
    let no_br_flags = no_br.clone();
    let line_chapters = chapter_lines
        .iter()
        .map(|(line_index, record)| (*line_index, chapters[*record].page_break_chapter))
        .collect::<Vec<_>>();
    let (fragment, emitted) = render_marked_lines(
        current.iter().map(String::as_str),
        &no_br_flags,
        &line_chapters,
        config,
        page_marker,
    );
    // Java: the TOC fragment (#kobo.N.M) is only used for chapters that are
    // not page-break chapters.
    for (line_index, id) in emitted {
        if let Some((_, record)) = chapter_lines.iter().find(|(index, _)| *index == line_index)
            && !chapters[*record].page_break_chapter
        {
            chapters[*record].anchor = Some(id);
        }
    }
    chapter_lines.clear();
    sections.push(fragment);
    current.clear();
    no_br.clear();
}

#[allow(clippy::too_many_arguments)]
fn append_section_line(
    line: &str,
    sections: &mut Vec<String>,
    current: &mut Vec<String>,
    no_br: &mut Vec<bool>,
    page_marker: &mut Option<&'static str>,
    section_index: &mut usize,
    chapter_lines: &mut Vec<(usize, usize)>,
    chapters: &mut [ChapterRecord],
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
            push_rendered_section(
                sections,
                current,
                no_br,
                chapter_lines,
                chapters,
                *page_marker,
                config,
            );
            *section_index += 1;
        }
        *page_marker = Some(PAGE_NO_CHAPTER_MARKER);
    }
    if should_force_page_break(current, line, config) {
        trim_trailing_empty_lines(current);
        no_br.truncate(current.len());
        if !current.is_empty() {
            push_rendered_section(
                sections,
                current,
                no_br,
                chapter_lines,
                chapters,
                *page_marker,
                config,
            );
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
/// Java `autoChapter`: 章の自動抽出が有効か。
fn auto_chapter_enabled(config: &AozoraConfig) -> bool {
    config.chapter_name_auto
        || config.chapter_num_only
        || config.chapter_num_title
        || config.chapter_num_paren
        || config.chapter_num_paren_title
}

/// Java `ChapterLineInfo.getLevel` の見出し種別 → レベル。
fn chapter_kind_for_level(level: u8) -> ChapterKind {
    match level {
        1 => ChapterKind::ChukiH1,
        2 => ChapterKind::ChukiH2,
        _ => ChapterKind::ChukiH3,
    }
}

const CHAPTER_NUM_CHARS: &str =
    "0123456789０１２３４５６７８９〇一二三四五六七八九十百壱弐参肆伍ⅠⅡⅢⅣⅤⅥⅦⅧⅨⅩⅪⅫ";
const CHAPTER_SEPARATORS: [char; 8] = [' ', '　', '-', '－', '「', '―', '『', '（'];
const CHAPTER_NAMES: [&str; 14] = [
    "プロローグ",
    "エピローグ",
    "モノローグ",
    "序",
    "序章",
    "序　章",
    "終章",
    "終　章",
    "間章",
    "間　章",
    "転章",
    "転　章",
    "幕間",
    "幕　間",
];
const CHAPTER_NUM_PREFIXES: [&str; 3] = ["第", "その", ""];
const CHAPTER_NUM_SUFFIXES: [&[&str]; 3] =
    [&["話", "章", "篇", "部", "節", "幕", "編"], &[""], &["章"]];
const CHAPTER_NUM_PAREN_PREFIXES: [&str; 4] = ["（", "〈", "〔", "【"];
const CHAPTER_NUM_PAREN_SUFFIXES: [&str; 4] = ["）", "〉", "〕", "】"];

fn is_chapter_num_char(character: char) -> bool {
    CHAPTER_NUM_CHARS.contains(character)
}

fn is_chapter_separator(character: char) -> bool {
    CHAPTER_SEPARATORS.contains(&character)
}

/// Java `removeSpace(removeTag(noRubyLine))`: ルビ・注記・タグを落として
/// 前後の空白を除去した行。
fn chapter_plain_line(line: &str, _config: &AozoraConfig) -> String {
    let no_ruby = crate::metadata::remove_ruby(line);
    let mut out = String::with_capacity(no_ruby.len());
    let chars = no_ruby.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '［' && chars.get(index + 1) == Some(&'＃') {
            // ［＃…］ 注記を除去
            let mut end = index + 2;
            while end < chars.len() && chars[end] != '］' {
                end += 1;
            }
            index = (end + 1).min(chars.len());
            continue;
        }
        if chars[index] == '<' {
            let mut end = index + 1;
            while end < chars.len() && chars[end] != '>' {
                end += 1;
            }
            index = (end + 1).min(chars.len());
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }
    out.trim_matches([' ', '　']).to_owned()
}

/// Java `getBookInfo` の章自動抽出。ヒットした種別のレベルを返す
/// (Java は 4 つの判定が独立なので 1 行で複数ヒットしうる)。
fn auto_chapter_levels(line: &str, config: &AozoraConfig) -> Vec<u8> {
    let chars = line.chars().collect::<Vec<_>>();
    let length = chars.len();
    let mut levels = Vec::new();

    if config.chapter_name_auto {
        for prefix in CHAPTER_NAMES {
            let prefix_chars = prefix.chars().count();
            if line.starts_with(prefix)
                && (length == prefix_chars || is_chapter_separator(chars[prefix_chars]))
            {
                levels.push(1);
                break;
            }
        }
    }

    if config.chapter_num_only || config.chapter_num_title {
        for (index, prefix) in CHAPTER_NUM_PREFIXES.iter().enumerate() {
            let prefix_chars = prefix.chars().count();
            if !line.starts_with(prefix) {
                continue;
            }
            let mut idx = prefix_chars;
            while idx < length && is_chapter_num_char(chars[idx]) {
                idx += 1;
            }
            if idx <= prefix_chars {
                break;
            }
            for suffix in CHAPTER_NUM_SUFFIXES[index] {
                let suffix_chars = suffix.chars().count();
                if idx + suffix_chars > length {
                    continue;
                }
                let after = idx + suffix_chars;
                let matches = if suffix_chars == 0 {
                    true
                } else {
                    chars[idx..after].iter().collect::<String>() == *suffix
                };
                if !matches {
                    continue;
                }
                if config.chapter_num_only && length == after
                    || config.chapter_num_title
                        && length > after
                        && is_chapter_separator(chars[after])
                {
                    levels.push(2);
                    break;
                }
            }
        }
    }

    // Java: prefix 無しの数字のみ / 数字+区切り も別途判定する
    if config.chapter_num_only || config.chapter_num_title {
        let mut idx = 0;
        while idx < length && is_chapter_num_char(chars[idx]) {
            idx += 1;
        }
        if idx > 0
            && (config.chapter_num_only && length == idx
                || config.chapter_num_title && length > idx && is_chapter_separator(chars[idx]))
        {
            levels.push(2);
        }
    }

    if config.chapter_num_paren || config.chapter_num_paren_title {
        for (index, prefix) in CHAPTER_NUM_PAREN_PREFIXES.iter().enumerate() {
            let prefix_chars = prefix.chars().count();
            if !line.starts_with(prefix) {
                continue;
            }
            let mut idx = prefix_chars;
            while idx < length && is_chapter_num_char(chars[idx]) {
                idx += 1;
            }
            if idx <= prefix_chars {
                break;
            }
            let suffix = CHAPTER_NUM_PAREN_SUFFIXES[index];
            let suffix_chars = suffix.chars().count();
            if idx + suffix_chars > length
                || chars[idx..idx + suffix_chars].iter().collect::<String>() != suffix
            {
                continue;
            }
            let after = idx + suffix_chars;
            if config.chapter_num_paren && length == after
                || config.chapter_num_paren_title
                    && length > after
                    && is_chapter_separator(chars[after])
            {
                levels.push(13);
                break;
            }
        }
    }

    levels
}

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
            // Java convertGaijiChuki は特殊文字 (※《》｜＃) の直前に内部マーカーを
            // 積む。マーカーは 《》｜＃ では \u0001 で後段の getChapterName が
            // 除去するが、米印 (※) はリテラルな ※ なので 2 文字残る。
            if replacement == "※" {
                cleaned.push('※');
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
    chapter_lines: &[(usize, bool)],
    config: &AozoraConfig,
    marker: Option<&str>,
) -> (String, Vec<(usize, String)>) {
    let (fragment, emitted) = render_lines(lines, no_br, chapter_lines, config);
    let fragment = marker
        .map(|marker| format!("{marker}\n{fragment}"))
        .unwrap_or(fragment);
    (fragment, emitted)
}

#[derive(Clone, Copy)]
struct HeadingSpec {
    element: &'static str,
    class_name: &'static str,
}

enum OpenBlock {
    Generated {
        close_tag: String,
        /// 字下げ系ブロックか（字下げ省略の対象）
        indent: bool,
    },
    Configured {
        fallback_close_tag: String,
        indent: bool,
    },
}

impl OpenBlock {
    fn is_indent(&self) -> bool {
        match self {
            OpenBlock::Generated { indent, .. } | OpenBlock::Configured { indent, .. } => *indent,
        }
    }

    /// ブロックを閉じるタグ。Java の `字下げ省略` (`</div>`) 相当。
    fn close_tag(&self) -> &str {
        match self {
            OpenBlock::Generated { close_tag, .. } => close_tag,
            OpenBlock::Configured {
                fallback_close_tag, ..
            } => fallback_close_tag,
        }
    }
}

/// Renders one section's lines to an XHTML fragment. `chapter_lines` maps
/// line indices to `page_break_chapter`; each chapter line gets a
/// `kobo.N.1` id; returns the fragment plus emitted (line_index, id) pairs.
fn render_lines<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    no_br: &[bool],
    chapter_lines: &[(usize, bool)],
    config: &AozoraConfig,
) -> (String, Vec<(usize, String)>) {
    let mut fragment = String::new();
    let mut has_line = false;
    let mut blocks: Vec<OpenBlock> = Vec::new();

    let mut pending_config_heading: Option<(String, String)> = None;
    // 本文が空になった行 (注記だけの行) が出した `<p><br/></p>` の位置。
    // セクション末尾に残ったものは Java では出力されないので取り除く。
    let mut empty_paragraphs: Vec<(usize, usize)> = Vec::new();
    let mut output_count = 0usize;
    let mut emitted: Vec<(usize, String)> = Vec::new();
    // Java の inYoko フィールド相当: ここから横組み〜ここで横組み終わり で切替
    let mut in_yoko = false;

    let block_markers = config
        .block_open_tags
        .keys()
        .chain(config.block_close_tags.keys())
        .map(|note| format!("［＃{note}］"))
        .collect::<Vec<_>>();
    let expanded_lines = merge_same_line_block_pieces(
        lines
            .into_iter()
            .enumerate()
            .flat_map(|(index, line)| {
                // Java: noBr 行は行全体を1バッファで注記→タグ置換して1行出力する。
                // ブロック注記での分割を行わず convert_inline に任せる。
                if no_br.get(index).copied().unwrap_or(false) {
                    vec![(index, line.to_owned())]
                } else {
                    split_block_notes(line, &block_markers)
                        .into_iter()
                        .map(move |piece| (index, piece))
                        .collect()
                }
            })
            .collect::<Vec<_>>(),
        config,
    );

    for (line_index, line) in expanded_lines
        .iter()
        .map(|(index, line)| (*index, line.as_str()))
    {
        has_line = true;
        let line_no_br = no_br.get(line_index).copied().unwrap_or(false);
        let page_break_chapter = chapter_lines
            .iter()
            .find(|(index, _)| *index == line_index)
            .map(|(_, pbc)| *pbc);
        let chapter_id = page_break_chapter.map(|_| format!("kobo.{}.1", output_count + 1));
        if let Some(id) = chapter_id.as_deref() {
            emitted.push((line_index, id.to_owned()));
        }
        if line_no_br {
            // Java: noBr 行は <p> で括らず行全体を1行出力する。
            // ブロック注記は convert_inline が inline_notes 経由でタグ化するが、
            // chuki_tag.txt に無い複合字下げ（ここから N 字下げ、折り返して M
            // 字下げ / N 字下げ、M 字詰め）だけはここでタグを差し込む。
            output_count += 1;
            // Java: 字下げブロック継続時は前の字下げブロックを閉じて同じ行で開く
            // (convertTextLineToEpub3 の `字下げ省略` → buf.append("</div>"))。
            let open_tag = indent_block_open_tag(line, config);
            let previous_close = match open_tag {
                Some(_) if blocks.iter().any(OpenBlock::is_indent) => {
                    blocks.pop().map(|block| block.close_tag().to_owned())
                }
                _ => None,
            };
            let mut converted = convert_no_br_block_notes(line, config, in_yoko, &mut blocks);
            if let (Some(open_tag), Some(close_tag)) = (open_tag, previous_close)
                && let Some(position) = converted.find(&open_tag)
            {
                converted.insert_str(position, &close_tag);
            }
            let converted = chapter_id
                .map(|id| inject_kobo_id(&converted, &id))
                .unwrap_or(converted);
            fragment.push_str(&converted);
            fragment.push('\n');
            // 分割を省略したため、ブロック開閉・横組み状態をここで追跡する
            for (note, _) in line_note_names(line) {
                if config.block_open_tags.contains_key(&note) {
                    if note.contains("横組み") {
                        in_yoko = true;
                    }
                    if let Some(open_tag) = config.block_open_tags.get(&note) {
                        blocks.push(OpenBlock::Configured {
                            fallback_close_tag: fallback_close_tag(open_tag),
                            indent: note.contains("字下げ"),
                        });
                    }
                } else if config.block_close_tags.contains_key(&note) {
                    if note.contains("横組み終わり") {
                        in_yoko = false;
                    }
                    // Java: キャプション終わりの </span> で画像ラッパーも閉じる
                    // (printLineBuffer の noBr 行でも同じ後始末を行う)
                    if config.block_close_tags.get(&note).map(String::as_str) == Some("</span>")
                        && image_wrapper_is_open(&fragment)
                    {
                        fragment.push_str("</span>");
                        fragment.push('\n');
                    }
                    blocks.pop();
                }
            }
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
            // Java: インライン字下げ注記でも字下げブロック継続時は前ブロックを閉じる
            let note =
                &line[start + "［＃".len()..line[start..].find('］').map_or(start, |o| start + o)];
            if note.contains("字下げ")
                && blocks.iter().any(OpenBlock::is_indent)
                && let Some(block) = blocks.pop()
            {
                let close = match block {
                    OpenBlock::Generated { close_tag, .. } => close_tag,
                    OpenBlock::Configured {
                        fallback_close_tag, ..
                    } => fallback_close_tag,
                };
                fragment.push_str(&close);
            }
            // Java は行全体を 1 バッファで処理するので、注記前の本文と
            // 開きタグの分だけ後続テキストの位置が後ろにずれる
            // (id は Java では変換後の printLineBuffer で付くため数えない)。
            let (prefix, prefix_len) = convert_inline_span(&line[..start], config, in_yoko, 0);
            fragment.push_str(&prefix);
            let content_offset = prefix_len + phase1_markup_len(&open_tag);
            let open_tag = chapter_id
                .map(|id| inject_kobo_id(&open_tag, &id))
                .unwrap_or(open_tag);
            fragment.push_str(&open_tag);
            fragment.push_str(&convert_inline_at(
                &line[end..],
                config,
                in_yoko,
                content_offset,
            ));
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
            fragment.push_str(&convert_inline_with_yoko(line, config, in_yoko));
            fragment.push_str(&close_tag);
            fragment.push('\n');
            continue;
        }

        if !blocks.is_empty() {
            if let Some((note, rest)) = heading_note_at_start(line)
                && !rest.trim().is_empty()
                && let Some((open_tag, close_tag)) = config.block_inline_tags.get(note)
            {
                output_count += 1;
                // Java: インライン字下げ注記でも字下げブロック継続時は前ブロックを閉じる
                if note.contains("字下げ")
                    && blocks.iter().any(OpenBlock::is_indent)
                    && let Some(block) = blocks.pop()
                {
                    let close = match block {
                        OpenBlock::Generated { close_tag, .. } => close_tag,
                        OpenBlock::Configured {
                            fallback_close_tag, ..
                        } => fallback_close_tag,
                    };
                    fragment.push_str(&close);
                }
                // Java: 注記前の行頭空白（leading）は convertEscapedText で無トリム出力される
                let leading_len = line.len() - line.trim_start().len();
                let content_offset =
                    phase1_text_len(&line[..leading_len]) + phase1_markup_len(open_tag);
                let open_tag = chapter_id
                    .map(|id| inject_kobo_id(open_tag, &id))
                    .unwrap_or_else(|| open_tag.clone());
                fragment.push_str(&line[..leading_len]);
                fragment.push_str(&open_tag);
                fragment.push_str(&convert_inline_at(rest, config, in_yoko, content_offset));
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
                    if let Some(OpenBlock::Generated { close_tag, .. }) = blocks.pop() {
                        output_count += 1;
                        let leading_len = line.len() - line.trim_start().len();
                        fragment.push_str(&line[..leading_len]);
                        fragment.push_str(&close_tag);
                        fragment.push('\n');
                    }
                    continue;
                }

                let closes_configured = matches!(blocks.last(), Some(OpenBlock::Configured { .. }));
                if closes_configured && let Some(close_tag) = config.block_close_tags.get(note) {
                    output_count += 1;
                    if note.contains("横組み終わり") {
                        in_yoko = false;
                    }
                    let leading_len = line.len() - line.trim_start().len();
                    fragment.push_str(&line[..leading_len]);
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
                    let leading_len = line.len() - line.trim_start().len();
                    // Java: 字下げブロック継続時は前の字下げブロックを閉じて同じ行で開く
                    if note.contains("字下げ")
                        && blocks.iter().any(OpenBlock::is_indent)
                        && let Some(OpenBlock::Generated {
                            close_tag: previous,
                            ..
                        }) = blocks.pop()
                    {
                        fragment.push_str(&line[..leading_len]);
                        fragment.push_str(&previous);
                        fragment.push_str(&open_tag);
                    } else {
                        fragment.push_str(&line[..leading_len]);
                        fragment.push_str(&open_tag);
                    }
                    fragment.push('\n');
                    blocks.push(OpenBlock::Generated {
                        close_tag,
                        indent: note.contains("字下げ"),
                    });
                    continue;
                }

                if let Some(open_tag) = config.block_open_tags.get(note) {
                    output_count += 1;
                    let leading_len = line.len() - line.trim_start().len();
                    // Java: 字下げブロック継続時は前の字下げブロックを閉じて同じ行で開く
                    if note.contains("字下げ")
                        && blocks.iter().any(OpenBlock::is_indent)
                        && let Some(OpenBlock::Configured {
                            fallback_close_tag, ..
                        }) = blocks.pop()
                    {
                        fragment.push_str(&line[..leading_len]);
                        fragment.push_str(&fallback_close_tag);
                        fragment.push_str(open_tag);
                    } else {
                        fragment.push_str(&line[..leading_len]);
                        fragment.push_str(open_tag);
                    }
                    fragment.push('\n');
                    blocks.push(OpenBlock::Configured {
                        fallback_close_tag: fallback_close_tag(open_tag),
                        indent: note.contains("字下げ"),
                    });
                    continue;
                }
                if let Some(tag) = config.block_single_tags.get(note) {
                    output_count += 1;
                    let leading_len = line.len() - line.trim_start().len();
                    fragment.push_str(&line[..leading_len]);
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
                    page_break_chapter.unwrap_or(false),
                    in_yoko,
                    &mut empty_paragraphs,
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
                    // Java: an empty heading note emits the empty heading tag
                    // on its own line; the next line is a normal paragraph.
                    append_heading(
                        &mut fragment,
                        spec,
                        "",
                        config,
                        &mut output_count,
                        chapter_id.as_deref(),
                        in_yoko,
                    );
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
                        in_yoko,
                    );
                }
                continue;
            }
            if let Some((open_tag, close_tag)) = generated_indent_block(note) {
                output_count += 1;
                let content_offset = phase1_markup_len(&open_tag);
                let open_tag = chapter_id
                    .map(|id| inject_kobo_id(&open_tag, &id))
                    .unwrap_or(open_tag);
                fragment.push_str(&open_tag);
                if !rest.trim().is_empty() {
                    fragment.push_str(&convert_inline_at(
                        rest.trim_start(),
                        config,
                        in_yoko,
                        content_offset,
                    ));
                    fragment.push('\n');
                } else {
                    fragment.push('\n');
                }
                blocks.push(OpenBlock::Generated {
                    close_tag,
                    indent: false,
                });
                continue;
            }
            if let Some(tag) = config.block_single_tags.get(note) {
                output_count += 1;
                let content_offset = phase1_markup_len(tag);
                let tag = chapter_id
                    .map(|id| inject_kobo_id(tag, &id))
                    .unwrap_or_else(|| tag.to_owned());
                fragment.push_str(&tag);
                if !rest.trim().is_empty() {
                    fragment.push_str(&convert_inline_at(
                        rest.trim_start(),
                        config,
                        in_yoko,
                        content_offset,
                    ));
                }
                fragment.push('\n');
                continue;
            }
            if let Some((open_tag, close_tag)) = config.block_inline_tags.get(note) {
                if rest.trim().is_empty() {
                    pending_config_heading = Some((open_tag.clone(), close_tag.clone()));
                } else {
                    output_count += 1;
                    // Java: 行頭の全角/半角空白はブロック開始タグの前に出力される
                    let leading_len = line.len() - line.trim_start().len();
                    let content_offset =
                        phase1_text_len(&line[..leading_len]) + phase1_markup_len(open_tag);
                    fragment.push_str(&line[..leading_len]);
                    let open_tag = chapter_id
                        .map(|id| inject_kobo_id(open_tag, &id))
                        .unwrap_or_else(|| open_tag.clone());
                    // Java: インライン字下げ注記でも字下げブロック継続時は前ブロックを閉じる
                    if note.contains("字下げ")
                        && blocks.iter().any(OpenBlock::is_indent)
                        && let Some(block) = blocks.pop()
                    {
                        let close = match block {
                            OpenBlock::Generated { close_tag, .. } => close_tag,
                            OpenBlock::Configured {
                                fallback_close_tag, ..
                            } => fallback_close_tag,
                        };
                        fragment.push_str(&close);
                    }
                    fragment.push_str(&open_tag);
                    fragment.push_str(&convert_inline_at(
                        rest.trim_start(),
                        config,
                        in_yoko,
                        content_offset,
                    ));
                    fragment.push_str(close_tag);
                    fragment.push('\n');
                }
                continue;
            }
            if let Some(open_tag) = config.block_open_tags.get(note) {
                output_count += 1;
                if note.contains("横組み") {
                    in_yoko = true;
                }
                let content_offset = phase1_markup_len(open_tag);
                let open_tag = chapter_id
                    .map(|id| inject_kobo_id(open_tag, &id))
                    .unwrap_or_else(|| open_tag.to_owned());
                let rest_trimmed = rest.trim();
                // 同一行クローズ（merge_same_line_block_pieces で結合された片）:
                // Java は行全体を1バッファで処理し、行頭空白＋開タグ＋内容＋閉タグ
                // を 1 行に出力する（<p> は付けない）。
                if !rest_trimmed.is_empty()
                    && let Some(close_note) = config
                        .block_close_tags
                        .keys()
                        .find(|close_note| rest_trimmed.ends_with(&format!("［＃{close_note}］")))
                {
                    let marker_len = format!("［＃{close_note}］").len();
                    let content = rest_trimmed[..rest_trimmed.len() - marker_len].trim_end();
                    let leading_len = line.len() - line.trim_start().len();
                    fragment.push_str(&line[..leading_len]);
                    fragment.push_str(&open_tag);
                    fragment.push_str(&convert_inline_at(
                        content,
                        config,
                        in_yoko,
                        phase1_text_len(&line[..leading_len]) + content_offset,
                    ));
                    fragment.push_str(config.block_close_tags.get(close_note).unwrap());
                    fragment.push('\n');
                    if close_note.contains("横組み終わり") {
                        in_yoko = false;
                    }
                    continue;
                }
                fragment.push_str(&open_tag);
                if !rest_trimmed.is_empty() {
                    fragment.push_str(&convert_inline_at(
                        rest_trimmed,
                        config,
                        in_yoko,
                        content_offset,
                    ));
                    fragment.push('\n');
                } else {
                    fragment.push('\n');
                }
                blocks.push(OpenBlock::Configured {
                    fallback_close_tag: fallback_close_tag(&open_tag),
                    indent: note.contains("字下げ"),
                });
                continue;
            }
        }

        append_line(
            &mut fragment,
            line,
            config,
            &mut output_count,
            chapter_id.as_deref(),
            page_break_chapter.unwrap_or(false),
            in_yoko,
            &mut empty_paragraphs,
        );
    }

    while let Some(block) = blocks.pop() {
        match block {
            OpenBlock::Generated { close_tag, .. } => {
                fragment.push_str(&close_tag);
                fragment.push('\n');
            }
            OpenBlock::Configured {
                fallback_close_tag, ..
            } => {
                fragment.push_str(&fallback_close_tag);
                fragment.push('\n');
            }
        }
    }
    if let Some((open_tag, close_tag)) = pending_config_heading {
        fragment.push_str(&open_tag);
        fragment.push_str(&close_tag);
        fragment.push('\n');
    }
    trim_trailing_empty_paragraphs(&mut fragment, &empty_paragraphs);

    if !has_line {
        fragment.push_str("    <p><br/></p>\n");
    }
    if config.ini.get_bool("MarkId").unwrap_or(false) {
        fragment = add_kobo_ids(&fragment);
    }
    (balance_xhtml(&fragment), emitted)
}

/// Java printLineBuffer: 空行は次の行を出力するときにまとめて `<p><br/></p>`
/// として出るため、セクション末尾に残った空行は出力されない。Lite は空行を
/// その場で出力するので、注記だけで空になった行の `<p><br/></p>` が
/// フラグメント末尾に接している場合だけ取り除く。
fn trim_trailing_empty_paragraphs(fragment: &mut String, marks: &[(usize, usize)]) {
    for (start, end) in marks.iter().rev() {
        if *end < fragment.len() || *start >= *end {
            break;
        }
        fragment.truncate(*start);
    }
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
    // ブロック注記の直前にある空白のみの片は注記と結合する
    // （Java: 「 ［＃ここから…］」→ 行頭空白＋ブロック開始タグが同じ行になる）
    let mut merged: Vec<String> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        if piece.starts_with("［＃")
            && let Some(last) = merged.last_mut()
            && last.trim().is_empty()
        {
            last.push_str(&piece);
        } else {
            merged.push(piece);
        }
    }
    let mut pieces = merged;
    if !rest.is_empty() || pieces.is_empty() {
        pieces.push(rest.to_owned());
    }
    pieces
}

/// 同一入力行に「開注記のみ片 + 内容片 + 閉注記片」が並ぶとき1片に結合する。
/// Java は行全体を1つのバッファで注記→タグ置換し、printLineBuffer の
/// isBlockTag 判定で <p> を付けずに 1 行出力する（例: `　　　<div
/// class="font-1em30">あ１</div>`）。Rust は行を分割して各片を独立処理
/// するため、この結合で 1 行化を再現する。
fn merge_same_line_block_pieces(
    pieces: Vec<(usize, String)>,
    config: &AozoraConfig,
) -> Vec<(usize, String)> {
    let mut merged: Vec<(usize, String)> = Vec::with_capacity(pieces.len());
    let mut i = 0;
    while i < pieces.len() {
        let (line_index, piece) = &pieces[i];
        let is_open_only = piece
            .trim()
            .strip_prefix("［＃")
            .and_then(|value| value.strip_suffix('］'))
            .is_some_and(|note| config.block_open_tags.contains_key(note));
        if is_open_only
            && let (Some((li2, content)), Some((li3, close_piece))) =
                (pieces.get(i + 1), pieces.get(i + 2))
        {
            let content_ok = li2 == line_index
                && !content.trim().is_empty()
                && !content.trim().starts_with("［＃");
            let close_note = close_piece
                .trim()
                .strip_prefix("［＃")
                .and_then(|value| value.strip_suffix('］'));
            if content_ok
                && li3 == line_index
                && close_note.is_some_and(|note| config.block_close_tags.contains_key(note))
            {
                let combined = format!("{piece}{content}{close_piece}");
                merged.push((*line_index, combined));
                i += 3;
                continue;
            }
        }
        merged.push((*line_index, piece.clone()));
        i += 1;
    }
    merged
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

/// noBr 行の本文をインライン変換する。Java は `chukiPattern` の複合字下げ
/// （ここから N 字下げ、折り返して M 字下げ / N 字下げ、M 字詰め）でも
/// タグをその位置に出力するが、`chuki_tag.txt` に定義が無いため
/// `convert_inline` ではタグ化されない。生成ブロックの注記だけを開きタグへ
/// 置き換え、それ以外の注記は行ごと1回の変換に任せる。
fn convert_no_br_block_notes(
    line: &str,
    config: &AozoraConfig,
    in_yoko: bool,
    blocks: &mut Vec<OpenBlock>,
) -> String {
    let mut output = String::new();
    let mut cursor = 0usize;
    let mut offset = 0usize;
    for (start, end, note) in generated_indent_notes(line) {
        let Some((open_tag, close_tag)) = generated_indent_block(&note) else {
            continue;
        };
        let (piece, consumed) = convert_inline_span(&line[cursor..start], config, in_yoko, offset);
        output.push_str(&piece);
        output.push_str(&open_tag);
        offset += consumed + phase1_markup_len(&open_tag);
        blocks.push(OpenBlock::Generated {
            close_tag,
            indent: note.contains("字下げ"),
        });
        cursor = end;
    }
    output.push_str(&convert_inline_at(&line[cursor..], config, in_yoko, offset));
    output
}

/// 行中の「字下げブロックを開く注記」の開きタグを返す。
/// `chuki_tag.txt` にある `ここからＮ字下げ` 系と、プログラム生成の
/// 複合字下げ（`generated_indent_block`）の両方を対象にする。
fn indent_block_open_tag(line: &str, config: &AozoraConfig) -> Option<String> {
    if let Some((_, _, note)) = generated_indent_notes(line).into_iter().next()
        && let Some((open_tag, _)) = generated_indent_block(&note)
    {
        return Some(open_tag);
    }
    line_note_names(line)
        .into_iter()
        .find(|(note, _)| note.ends_with("字下げ"))
        .and_then(|(note, _)| {
            config.block_open_tags.get(&note).cloned().or_else(|| {
                config
                    .block_inline_tags
                    .get(&note)
                    .map(|(open, _)| open.clone())
            })
        })
}

/// 行中の複合字下げ注記を (開始, 終了, 注記名) で列挙する。
fn generated_indent_notes(line: &str) -> Vec<(usize, usize, String)> {
    let mut notes = Vec::new();
    for (note, end) in line_note_names(line) {
        if generated_indent_block(&note).is_none() {
            continue;
        }
        let start = end - note.len() - "［＃］".len();
        notes.push((start, end, note));
    }
    notes
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
        // Java は 罫囲み / 枠囲み の各組で「破線 → 実線」を else-if で排他にする
        // (AozoraEpub3Converter.java:2277-2283)。`破線枠囲み` は `枠囲み` を含むため、
        // 独立した contains 判定にすると `border` が余計に付く。
        for (dashed, solid) in [("破線罫囲み", "罫囲み"), ("破線枠囲み", "枠囲み")]
        {
            if rest.contains(dashed) {
                classes.push("dashed_border".to_owned());
            } else if rest.contains(solid) {
                classes.push("border".to_owned());
            }
        }
        for (needle, class) in [("中央揃え", "center"), ("横書き", "yoko")] {
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
    in_yoko: bool,
) {
    *output_count += 1;
    let mut open_tag = format!("<{} class=\"{}\">", spec.element, spec.class_name);
    // 見出しの開きタグもフェーズ1バッファに入る (Java は行全体を1バッファで
    // 処理する)。id は Java では変換後の printLineBuffer で付くため数えない。
    let content_offset = phase1_markup_len(&open_tag);
    if let Some(id) = chapter_id {
        open_tag = inject_kobo_id(&open_tag, id);
    }
    fragment.push_str(&open_tag);
    fragment.push_str(&convert_inline_at(text, config, in_yoko, content_offset));
    fragment.push_str("</");
    fragment.push_str(spec.element);
    fragment.push_str(">\n");
}

#[allow(clippy::too_many_arguments)]
fn append_line(
    fragment: &mut String,
    line: &str,
    config: &AozoraConfig,
    output_count: &mut usize,
    chapter_id: Option<&str>,
    page_break_chapter: bool,
    in_yoko: bool,
    empty_paragraphs: &mut Vec<(usize, usize)>,
) {
    let converted = convert_inline_with_yoko(line, config, in_yoko);
    if append_open_image_line(fragment, &converted) {
        return;
    }
    if converted.trim().is_empty() {
        // Java printLineBuffer は空バッファを空行として数え、次の行の出力時に
        // まとめて `<p><br/></p>` を出す。Lite はその場で1行出し、セクション
        // 末尾に残った空行は後段で取り除く (trim_trailing_empty_paragraphs)。
        if line.trim().is_empty() {
            fragment.push_str("    <p><br/></p>\n");
        } else {
            // 注記だけで本文が空になった行 (記録して末尾なら取り除く)
            let start = fragment.len();
            fragment.push_str("    <p><br/></p>\n");
            empty_paragraphs.push((start, fragment.len()));
        }
    } else {
        *output_count += 1;
        // 見出し注記で生成された h1/h2/h3 は <p> で包まない（Java 準拠）
        let is_heading = ["<h1", "<h2", "<h3"]
            .iter()
            .any(|tag| converted.contains(tag));
        if is_heading {
            // Java blockTag path: the chapter id goes on the heading tag.
            let converted = chapter_id
                .map(|id| inject_kobo_id(&converted, id))
                .unwrap_or(converted);
            fragment.push_str(&converted);
            fragment.push('\n');
        } else {
            // Java: <p id> only for non-page-break chapters.
            match chapter_id.filter(|_| !page_break_chapter) {
                Some(id) => {
                    fragment.push_str("    <p id=\"");
                    fragment.push_str(id);
                    fragment.push_str("\">");
                }
                None => fragment.push_str("    <p>"),
            }
            fragment.push_str(&converted);
            fragment.push_str("</p>\n");
        }
    }
}
#[allow(clippy::too_many_arguments)]
fn append_block_line(
    fragment: &mut String,
    line: &str,
    config: &AozoraConfig,
    output_count: &mut usize,
    chapter_id: Option<&str>,
    page_break_chapter: bool,
    in_yoko: bool,
    empty_paragraphs: &mut Vec<(usize, usize)>,
) {
    let converted = convert_inline_with_yoko(line, config, in_yoko);
    if append_open_image_line(fragment, &converted) {
        return;
    }
    if converted.trim().is_empty() {
        // append_line と同じ (Java printLineBuffer の空行扱い)。
        if line.trim().is_empty() {
            fragment.push_str("<p><br/></p>\n");
        } else {
            let start = fragment.len();
            fragment.push_str("<p><br/></p>\n");
            empty_paragraphs.push((start, fragment.len()));
        }
    } else {
        *output_count += 1;
        match chapter_id.filter(|_| !page_break_chapter) {
            Some(id) => {
                fragment.push_str("<p id=\"");
                fragment.push_str(id);
                fragment.push_str("\">");
            }
            None => fragment.push_str("<p>"),
        }
        fragment.push_str(&converted);
        fragment.push_str("</p>\n");
    }
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

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;
