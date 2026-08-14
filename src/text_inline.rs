use super::{AozoraConfig, heading_spec};
const WRC_BREAK_MARKER: char = '\u{0001}';
const EMPTY_NOTE_KIND: u8 = 0xFF;

pub(super) fn convert_inline(input: &str, config: &AozoraConfig) -> String {
    convert_inline_with_options(input, config, true, true)
}

fn convert_inline_without_auto_yoko(input: &str, config: &AozoraConfig) -> String {
    convert_inline_with_options(input, config, false, true)
}

fn convert_inline_with_options(
    input: &str,
    config: &AozoraConfig,
    auto_yoko: bool,
    allow_upright: bool,
) -> String {
    let input = rewrite_character_replacements(input, config);
    // エスケープペア: ＜＜→※《 ＞＞→※》 <<→※《 >>→※》 (Java convertEscapedText)
    // ※マーカーはループ内のエスケープ分岐で除去される
    let input = rewrite_escape_pairs(&input);
    // くの字点: ／＼→〳〵 ／″＼→〴〵 (Java convertGaijiChuki)
    let input = rewrite_kunoji_point(&input);
    let input = rewrite_suffix_notes(&input, config);
    let input = rewrite_warichu_breaks(&input);
    let input = rewrite_alternative_gaiji(&input, config);
    let input = if auto_yoko {
        rewrite_auto_yoko(&input, config)
    } else {
        input
    };
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0;
    let mut tcy_depth = 0usize;
    let mut link_started = false;
    let mut implicit_ruby_open = false;
    while index < chars.len() {
        if implicit_ruby_open
            && chars[index] != '《'
            && ruby_base_kind(chars[index]).is_none()
            && !image_note_followed_by_ruby(&chars, index)
        {
            output.push_str("</ruby>");
            implicit_ruby_open = false;
        }
        if chars[index] == WRC_BREAK_MARKER {
            output.push_str(
                config
                    .inline_notes
                    .get("改行")
                    .map(String::as_str)
                    .unwrap_or("<br/>"),
            );
            index += 1;
            continue;
        }
        // エスケープ文字: ※の直後の《》｜＃※ はルビ/注記処理しない
        // (Java convertReplacedChar: ※を削除して文字を素通し出力)
        if matches!(chars[index], '《' | '》' | '｜' | '＃' | '※')
            && index > 0
            && chars[index - 1] == '※'
        {
            if output.ends_with('※') {
                output.pop();
            }
            push_text_char_escaped(&mut output, chars[index]);
            index += 1;
            continue;
        }
        if chars[index] == '※'
            && let Some((end, replacement)) = parse_image_note(&chars, index + 1, config)
        {
            // Java: ※付きの画像注記（※［＃…（img/…）入る］）は画像を出力しない
            // （※ は外字注記開始として消費され、注記本体は画像注記として処理されない）
            // ただし #GAIJI# フラグ付き（外字画像）は出力する
            if !chars[index + 1..end]
                .iter()
                .collect::<String>()
                .contains("#GAIJI#")
            {
                let _ = replacement;
                index = end;
                continue;
            }
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if chars[index] == '※'
            && let Some((end, replacement)) =
                parse_gaiji_note(&chars, index, config, allow_upright && tcy_depth == 0)
        {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if chars[index] == '※'
            && let Some((end, replacement)) = parse_unicode_note(&chars, index + 1, config)
        {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_unicode_note(&chars, index, config) {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_image_note(&chars, index, config) {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_inline_heading(&chars, index, config) {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_configured_inline_block(&chars, index, config) {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_inline_note(&chars, index, config) {
            if is_tcy_open(&replacement, config) {
                tcy_depth += 1;
            } else if is_tcy_close(&replacement, config) {
                tcy_depth = tcy_depth.saturating_sub(1);
            }
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if chars[index] == '<'
            && let Some((end, markup)) = parse_configured_markup(&chars, index, config)
        {
            if is_tcy_open(&markup, config) {
                tcy_depth += 1;
            } else if is_tcy_close(&markup, config) {
                tcy_depth = tcy_depth.saturating_sub(1);
            }
            output.push_str(&markup);
            index = end;
            continue;
        }
        if chars[index] == '<'
            && let Some((end, replacement)) = parse_raw_image(&chars, index, config)
        {
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if chars[index] == '<'
            && let Some((end, replacement)) = parse_raw_anchor(&chars, index)
        {
            // Java: linkStarted フラグで開閉を追跡（破棄された <a> は閉じも破棄）
            if replacement == "</a>" {
                if link_started {
                    link_started = false;
                    output.push_str("</a>");
                }
            } else if replacement.is_empty() {
                link_started = false;
            } else {
                link_started = true;
                output.push_str(&replacement);
            }
            index = end;
            continue;
        }
        if chars[index] == '〔'
            && let Some(close) = find_closing_latin_bracket(&chars, index)
        {
            let inner = &chars[index + 1..close];
            if !inner.is_empty() && inner.iter().copied().all(is_half_space) {
                let separated = inner.iter().collect::<String>();
                let replacement = convert_latin(&separated, config);
                output.push_str(&escape_text(&replacement));
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
                let continues = has_following_implicit_ruby(&chars, close + 1);
                if continues {
                    output.push_str("<ruby>");
                    push_ruby_part(
                        &mut output,
                        &base,
                        &reading,
                        config,
                        auto_yoko && tcy_depth == 0,
                    );
                    implicit_ruby_open = true;
                } else {
                    push_ruby(
                        &mut output,
                        &base,
                        &reading,
                        config,
                        auto_yoko && tcy_depth == 0,
                    );
                }
                index = close + 1;
                continue;
            }
        }

        if chars[index] == '《'
            && let Some(close) = find_closing_ruby(&chars, index)
            && index > 0
            && (ruby_base_kind(chars[index - 1]).is_some()
                || latin_bracket_start_ending_at(&chars, index).is_some()
                || image_note_start_ending_at(&chars, index).is_some()
                || gaiji_note_start_ending_at(&chars, index)
                    .is_some_and(|note_start| {
                        note_rendered_kind(&chars[note_start..index], config).is_some_and(|kind| kind != EMPTY_NOTE_KIND)
                    })
                || unicode_note_start_ending_at(&chars, index, config).is_some_and(|note_start| {
                    note_rendered_kind(&chars[note_start..index], config)
                        .is_some_and(|kind| kind != EMPTY_NOTE_KIND)
                }))
        {
            let bracket_start = latin_bracket_start_ending_at(&chars, index);
            let mut base_start = bracket_start
                .map(|start| {
                    // Java: 英字ランは直前の半角空白も含む
                    if start > 0 && is_half_space(chars[start - 1]) {
                        start - 1
                    } else {
                        start
                    }
                })
                .or_else(|| image_note_start_ending_at(&chars, index))
                .or_else(|| gaiji_note_start_ending_at(&chars, index))
                .or_else(|| unicode_note_start_ending_at(&chars, index, config))
                .unwrap_or(index - 1);
            // 現在のラン種別。注記をまたぐ場合は注記の描画種別で継続する
            // (Java: 基底はルビ直前の文字種ラン。外字→漢字等の基底文字なら注記もランに含む)。
            let mut run_kind = if bracket_start.is_some() {
                Some(3)
            } else if let Some(note_start) = gaiji_note_start_ending_at(&chars, index)
                .or_else(|| unicode_note_start_ending_at(&chars, index, config))
            {
                // 注記始まりの基底は注記の描画種別から始める
                note_rendered_kind(&chars[note_start..index], config)
            } else {
                ruby_base_kind(chars[base_start])
            };
            loop {
                if let Some(note_start) = gaiji_note_start_ending_at(&chars, base_start) {
                    let note_kind = note_rendered_kind(&chars[note_start..base_start], config);
                    if note_kind == Some(EMPTY_NOTE_KIND) {
                        base_start = note_start;
                        continue;
                    }
                    let Some(note_kind) = note_kind else {
                        break;
                    };
                    base_start = note_start;
                    run_kind = Some(note_kind);
                    continue;
                }
                if let Some(note_start) = image_note_start_ending_at(&chars, base_start) {
                    let note_kind = note_rendered_kind(&chars[note_start..base_start], config);
                    if note_kind == Some(EMPTY_NOTE_KIND) {
                        base_start = note_start;
                        continue;
                    }
                    let Some(note_kind) = note_kind else {
                        break;
                    };
                    base_start = note_start;
                    run_kind = Some(note_kind);
                    continue;
                }
                if let Some(note_start) = unicode_note_start_ending_at(&chars, base_start, config) {
                    let note_kind = note_rendered_kind(&chars[note_start..base_start], config);
                    if note_kind == Some(EMPTY_NOTE_KIND) {
                        base_start = note_start;
                        continue;
                    }
                    let Some(note_kind) = note_kind else {
                        break;
                    };
                    base_start = note_start;
                    run_kind = Some(note_kind);
                    continue;
                }
                if base_start == 0 {
                    break;
                }
                let Some(previous_kind) = ruby_base_kind(chars[base_start - 1]) else {
                    break;
                };
                if run_kind.is_some_and(|kind| kind != previous_kind) {
                    break;
                }
                base_start -= 1;
            }
            let base = chars[base_start..index].iter().collect::<String>();
            let rendered_base = if auto_yoko && tcy_depth == 0 {
                convert_inline(&base, config)
            } else {
                convert_inline_without_auto_yoko(&base, config)
            };
            if output.ends_with(&rendered_base) {
                output.truncate(output.len() - rendered_base.len());
            }
            let reading = chars[index + 1..close].iter().collect::<String>();
            let continues = has_following_implicit_ruby(&chars, close + 1);
            if !implicit_ruby_open {
                output.push_str("<ruby>");
            }
            push_ruby_part(
                &mut output,
                &base,
                &reading,
                config,
                auto_yoko && tcy_depth == 0,
            );
            implicit_ruby_open = continues;
            if !continues {
                output.push_str("</ruby>");
            }
            index = close + 1;
            continue;
        }
        if chars[index] == '《'
            && let Some(close) = find_closing_ruby(&chars, index)
        {
            // Java: ルビ開始文字無しの《》は警告して破棄する
            index = close + 1;
            continue;
        }
        index += push_text_char(
            &mut output,
            &chars,
            index,
            config,
            allow_upright && tcy_depth == 0,
        );
    }
    if implicit_ruby_open {
        output.push_str("</ruby>");
    }
    output
}
/// ＜＜・＞＞・<<・>> の2連続を ※《・※》・※《・※》 へ変換する。
/// 3連続以上の連続では変換しない（Java convertEscapedText と同じ条件）。
fn rewrite_escape_pairs(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < chars.len() {
        let character = chars[index];
        let is_open = matches!(character, '＜' | '<');
        let is_close = matches!(character, '＞' | '>');
        if (is_open || is_close)
            && chars.get(index + 1) == Some(&character)
            && chars.get(index + 2) != Some(&character)
            && (index == 0 || chars.get(index - 1) != Some(&character))
        {
            output.push('※');
            output.push(if is_open { '《' } else { '》' });
            index += 2;
            continue;
        }
        output.push(character);
        index += 1;
    }
    output
}

/// ／＼→〳〵 ／″＼→〴〵 (くの字点)
fn rewrite_kunoji_point(input: &str) -> String {
    input.replace("／″＼", "〴〵").replace("／＼", "〳〵")
}

fn rewrite_warichu_breaks(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < chars.len() {
        let Some((open_end, note)) = note_range(&chars, index) else {
            output.push(chars[index]);
            index += 1;
            continue;
        };
        if !note.ends_with("割り注") || note.ends_with("終わり") {
            output.extend(chars[index..open_end].iter());
            index = open_end;
            continue;
        }
        let close_note = if note.starts_with("ここから") {
            "ここで割り注終わり"
        } else {
            "割り注終わり"
        };
        let Some((close_start, close_end)) = find_note(&chars, open_end, close_note) else {
            output.extend(chars[index..open_end].iter());
            index = open_end;
            continue;
        };
        let (body_start, body_end, moved_brackets) = if (chars[open_end] == '〔'
            && chars.get(close_start.saturating_sub(1)) == Some(&'〕')
            || chars[open_end] == '（' && chars.get(close_start.saturating_sub(1)) == Some(&'）'))
            && (index == 0 || !matches!(chars[index - 1], '〔' | '（'))
        {
            (open_end + 1, close_start - 1, true)
        } else {
            (open_end, close_start, false)
        };
        if moved_brackets {
            output.push(chars[open_end]);
        }
        output.extend(chars[index..open_end].iter());
        output.push_str(&rewrite_warichu_body(&chars[body_start..body_end]));
        output.extend(chars[close_start..close_end].iter());
        if moved_brackets {
            output.push(chars[close_start - 1]);
        }
        index = close_end;
    }
    output
}

fn rewrite_warichu_body(body: &[char]) -> String {
    let explicit_break = body.iter().enumerate().find_map(|(index, _)| {
        let (end, note) = note_range(body, index)?;
        (note == "改行").then_some((index, end))
    });
    if let Some((break_start, break_end)) = explicit_break {
        let mut output = String::with_capacity(body.len());
        output.extend(body[..break_start].iter());
        output.push(WRC_BREAK_MARKER);
        output.extend(body[break_end..].iter());
        return output;
    }

    let mut units = Vec::new();
    let mut index = 0;
    while index < body.len() {
        if let Some((end, _)) = note_range(body, index) {
            index = end;
            continue;
        }
        if body[index] == '｜' {
            index += 1;
            continue;
        }
        if body[index] == '《'
            && let Some(close) = find_closing_ruby(body, index)
        {
            index = close + 1;
            continue;
        }
        let width = if is_halfwidth_for_warichu(body[index]) {
            1
        } else {
            2
        };
        units.push((index, width));
        index += 1;
    }
    if units.len() < 2 {
        return body.iter().collect();
    }
    let total = units.iter().map(|(_, width)| *width).sum::<usize>();
    let half = total.div_ceil(2);
    let mut width = 0;
    let break_index = units.iter().find_map(|(index, unit_width)| {
        if width >= half {
            Some(*index)
        } else {
            width += *unit_width;
            None
        }
    });
    let Some(mut break_index) = break_index.filter(|index| *index > 0) else {
        return body.iter().collect();
    };
    if matches!(body.get(break_index), Some('、' | '。')) {
        break_index += 1;
    }
    let mut output = String::with_capacity(body.len() + 1);
    output.extend(body[..break_index].iter());
    output.push(WRC_BREAK_MARKER);
    output.extend(body[break_index..].iter());
    output
}

fn rewrite_character_replacements(input: &str, config: &AozoraConfig) -> String {
    if config.character_replacements.is_empty() {
        return input.to_owned();
    }
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < chars.len() {
        // Single-character rules are applied per character at output time
        // (like the reference replaceMap) so replacement results are not
        // re-normalized; only the two-character rules run as a pre-scan.
        let replacement = chars.get(index..index + 2).and_then(|candidate| {
            let key = candidate.iter().collect::<String>();
            (key.chars().count() == 2)
                .then(|| config.character_replacements.get(&key))
                .flatten()
                .map(|value| (2, value.as_str()))
        });
        if let Some((length, replacement)) = replacement {
            output.push_str(replacement);
            index += length;
        } else {
            output.push(chars[index]);
            index += 1;
        }
    }
    output
}

fn rewrite_suffix_notes(input: &str, config: &AozoraConfig) -> String {
    let mut current = input.to_owned();

    loop {
        if let Some(rewritten) = rewrite_special_suffix_once(&current) {
            current = rewritten;
            continue;
        }

        let chars = current.chars().collect::<Vec<_>>();
        let mut index = 0;
        let mut selected = None;

        while index < chars.len() {
            if let Some((end, target, suffix)) = suffix_note_at(&chars, index) {
                if let Some(rule) = config.suffix_notes.get(&suffix) {
                    let prefix = chars[..index].iter().collect::<String>();
                    if suffix_target_range(&prefix, &target).is_some() && selected.is_none() {
                        selected = Some((
                            index,
                            end,
                            target,
                            rule.start.clone(),
                            rule.end.clone(),
                        ));
                    }
                }
                index = end;
            } else {
                index += 1;
            }
        }

        let Some((start, end, target, start_tag, end_tag)) = selected else {
            return current;
        };
        let prefix = chars[..start].iter().collect::<String>();
        let suffix = chars[end..].iter().collect::<String>();
        let start_note = format!("［＃{start_tag}］");
        let end_note = format!("［＃{end_tag}］");
        // Java: 注記の後ろにルビがあれば前に移動して位置を調整する。
        // 移動後の文字列で対象位置を計算すると ｜ を含めた span になる。
        let ruby_end = suffix.strip_prefix('《').and_then(|reading| {
            reading
                .find('》')
                .map(|offset| '《'.len_utf8() + offset + '》'.len_utf8())
        });
        let (target_start, _) = if let Some(ruby_end) = ruby_end {
            let mut range_prefix = prefix.clone();
            range_prefix.push_str(&suffix[..ruby_end]);
            suffix_target_range(&range_prefix, &target).unwrap()
        } else {
            suffix_target_range(&prefix, &target).unwrap()
        };
        let mut rewritten = prefix;
        rewritten.insert_str(target_start, &start_note);
        if let Some(ruby_end) = ruby_end {
            rewritten.push_str(&suffix[..ruby_end]);
            rewritten.push_str(&end_note);
            rewritten.push_str(&suffix[ruby_end..]);
        } else {
            rewritten.push_str(&end_note);
            rewritten.push_str(&suffix);
        }
        current = rewritten;
    }
}

fn rewrite_special_suffix_once(input: &str) -> Option<String> {
    let chars = input.chars().collect::<Vec<_>>();
    for start in 0..chars.len() {
        let Some((end, target, suffix)) = suffix_note_at(&chars, start) else {
            continue;
        };
        let prefix = chars[..start].iter().collect::<String>();
        let replacement = if let Some(reading) = suffix
            .strip_prefix("に「")
            .and_then(|value| value.strip_suffix("」のルビ"))
            .filter(|reading| !reading.is_empty() && !reading.starts_with("ママ"))
        {
            // Java: targetLength は target 全体（ルビ込み）でカウント。
            // 対象に《》が含まれると可視文字が足りず先頭(0)になる。
            let (target_start, target_end) =
                suffix_target_range_by_len(&prefix, target.chars().count())?;
            let mut rewritten = String::with_capacity(prefix.len() + reading.len() + 4);
            rewritten.push_str(&prefix[..target_start]);
            rewritten.push('｜');
            rewritten.push_str(&prefix[target_start..target_end]);
            rewritten.push('《');
            rewritten.push_str(reading);
            rewritten.push('》');
            rewritten.push_str(&prefix[target_end..]);
            rewritten
        } else if suffix
            .strip_prefix("の左に「")
            .and_then(|value| value.strip_suffix("」のルビ"))
            .is_some()
        {
            prefix
        } else if suffix.ends_with("のルビ付き終わり") {
            let mut rewritten = prefix;
            if let Some(marker_start) = rewritten.rfind("［＃左にルビ付き］") {
                rewritten
                    .replace_range(marker_start..marker_start + "［＃左にルビ付き］".len(), "");
            }
            rewritten
        } else if suffix.ends_with("の注記付き終わり") {
            let mut rewritten = prefix;
            if let Some(marker_start) = rewritten.rfind("［＃左に注記付き］") {
                rewritten
                    .replace_range(marker_start..marker_start + "［＃左に注記付き］".len(), "");
            } else {
                if let Some(marker_start) = rewritten.rfind("［＃注記付き］") {
                    rewritten
                        .replace_range(marker_start..marker_start + "［＃注記付き］".len(), "｜");
                }
                rewritten.push('《');
                rewritten.push_str(&target);
                rewritten.push('》');
            }
            rewritten
        } else {
            continue;
        };
        let byte_end = chars[..end]
            .iter()
            .map(|character| character.len_utf8())
            .sum::<usize>();
        return Some(format!("{replacement}{}", &input[byte_end..]));
    }
    None
}

fn rewrite_alternative_gaiji(input: &str, config: &AozoraConfig) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < chars.len() {
        if let Some((end, note)) = gaiji_note_range(&chars, index) {
            let bare_note = note
                .strip_prefix("※［＃")
                .and_then(|value| value.strip_suffix('］'))
                .unwrap_or(&note);
            let key = bare_note.split(['、', ',']).next().unwrap_or(bare_note);
            let normalized_key = crate::config::normalize_gaiji_key(key);
            let key_note = format!("※［＃{key}］");
            let normalized_key_note = format!("※［＃{normalized_key}］");
            let normalized_note = crate::config::normalize_gaiji_key(&note);
            if let Some(replacement) = config
                .gaiji_alternatives
                .get(&note)
                .or_else(|| config.gaiji_alternatives.get(&normalized_note))
                .or_else(|| config.gaiji_alternatives.get(bare_note))
                .or_else(|| config.gaiji_alternatives.get(key))
                .or_else(|| config.gaiji_alternatives.get(&normalized_key))
                .or_else(|| config.gaiji_alternatives.get(&key_note))
                .or_else(|| config.gaiji_alternatives.get(&normalized_key_note))
            {
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
fn contains_literal_gaiji_note(input: &str) -> bool {
    let mut remainder = input;
    while let Some(start) = remainder.find("※［＃") {
        let after_open = &remainder[start + "※［＃".len()..];
        let Some(close) = after_open.find('］') else {
            return false;
        };
        let note = &after_open[..close];
        if !note.contains('（') && !note.contains("#GAIJI#") {
            return true;
        }
        remainder = &after_open[close + '］'.len_utf8()..];
    }
    false
}

fn convert_ruby_reading(reading: &str, config: &AozoraConfig) -> String {
    convert_inline_with_options(reading, config, false, false)
}
fn rewrite_auto_yoko(input: &str, config: &AozoraConfig) -> String {
    if !config.vertical || !config.auto_yoko {
        return input.to_owned();
    }
    let open = config
        .inline_notes
        .get("縦中横")
        .map(String::as_str)
        .unwrap_or("<span class=\"tcy\">");
    let close = config
        .inline_notes
        .get("縦中横終わり")
        .map(String::as_str)
        .unwrap_or("</span>");
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    let mut explicit_tcy = false;

    while index < chars.len() {
        if chars[index] == '［'
            && chars.get(index + 1) == Some(&'＃')
            && let Some(close_index) = chars
                .iter()
                .enumerate()
                .skip(index + 2)
                .find_map(|(candidate, character)| (*character == '］').then_some(candidate))
        {
            let note = chars[index + 2..close_index].iter().collect::<String>();
            output.extend(chars[index..=close_index].iter());
            explicit_tcy = match note.as_str() {
                "縦中横" => true,
                "縦中横終わり" => false,
                _ => explicit_tcy,
            };
            index = close_index + 1;
            continue;
        }
        if explicit_tcy {
            output.push(chars[index]);
            index += 1;
            continue;
        }
        if chars[index] == '<'
            && let Some(close_index) = chars
                .iter()
                .enumerate()
                .skip(index + 1)
                .find_map(|(candidate, character)| (*character == '>').then_some(candidate))
        {
            output.extend(chars[index..=close_index].iter());
            index = close_index + 1;
            continue;
        }
        if chars[index] == '｜'
            && let Some((open_index, close_index)) = find_ruby_bounds(&chars, index + 1)
            && contains_literal_gaiji_note(&chars[index + 1..open_index].iter().collect::<String>())
        {
            output.extend(chars[index..=close_index].iter());
            index = close_index + 1;
            continue;
        }
        if chars[index] == '《'
            && let Some(close_index) = chars
                .iter()
                .enumerate()
                .skip(index + 1)
                .find_map(|(candidate, character)| (*character == '》').then_some(candidate))
        {
            output.extend(chars[index..=close_index].iter());
            index = close_index + 1;
            continue;
        }

        let is_digit = chars[index].is_ascii_digit();
        let is_equation = matches!(chars[index], '!' | '?');
        if is_digit || is_equation {
            let run_end = chars
                .iter()
                .enumerate()
                .skip(index)
                .find_map(|(candidate, character)| {
                    let same_kind = if is_digit {
                        character.is_ascii_digit()
                    } else {
                        matches!(character, '!' | '?')
                    };
                    (!same_kind).then_some(candidate)
                })
                .unwrap_or(chars.len());
            let run_length = run_end - index;
            let take = if is_digit {
                if run_length >= 3 && config.auto_yoko_num3 {
                    3
                } else if run_length == 2 {
                    2
                } else if run_length == 1 && config.auto_yoko_num1 {
                    1
                } else {
                    0
                }
            } else if run_length >= 3 && config.auto_yoko_eq3 {
                3
            } else if run_length == 2 {
                2
            } else if run_length == 1 && config.auto_yoko_eq1 {
                1
            } else {
                0
            };
            if take > 0
                && !image_note_follows(&chars, run_end)
                && tcy_boundary_before(&chars, index)
                && tcy_boundary_after(&chars, index + take)
            {
                output.push_str(open);
                output.extend(chars[index..index + take].iter());
                output.push_str(close);
                index += take;
                continue;
            }
        }

        output.push(chars[index]);
        index += 1;
    }
    output
}
fn image_note_follows(chars: &[char], index: usize) -> bool {
    let start = if chars.get(index) == Some(&'※') {
        index + 1
    } else {
        index
    };
    image_note_parts(chars, start).is_some()
}

fn tcy_boundary_before(chars: &[char], index: usize) -> bool {
    let mut index = index;
    while index > 0 {
        index -= 1;
        if chars[index].is_ascii_whitespace() {
            continue;
        }
        return !is_halfwidth_for_tcy(chars[index]);
    }
    true
}

fn tcy_boundary_after(chars: &[char], mut index: usize) -> bool {
    while index < chars.len() {
        if chars[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        return !is_halfwidth_for_tcy(chars[index]);
    }
    true
}

fn is_halfwidth_for_tcy(character: char) -> bool {
    character.is_ascii() || (('\u{ff61}'..='\u{ff9f}').contains(&character))
}
fn is_halfwidth_for_warichu(character: char) -> bool {
    ('\u{21}'..='\u{2af}').contains(&character)
}

fn suffix_note_at(chars: &[char], start: usize) -> Option<(usize, String, String)> {
    if chars.get(start) != Some(&'［') || chars.get(start + 1) != Some(&'＃') {
        return None;
    }
    let target_start = {
        let mut found = None;
        for (index, character) in chars.iter().enumerate().skip(start + 2) {
            match character {
                '「' => {
                    found = Some(index);
                    break;
                }
                '］' => break,
                _ => {}
            }
        }
        found?
    };
    let target_end = {
        // Java chukiSufPattern「([^］]+)」: 対象は ］ までの最後の 」 まで（「」入れ子）。
        // ただし suffix 側に「…」ペアがある注記（に「読み」のルビ等）は最初の 」 で区切る。
        let first_close = chars
            .iter()
            .enumerate()
            .skip(target_start + 1)
            .find_map(|(index, character)| (*character == '」').then_some(index))?;
        let suffix_part = chars[first_close + 1..]
            .iter()
            .take_while(|character| **character != '］')
            .collect::<String>();
        if suffix_part.contains('「') {
            first_close
        } else {
            chars
                .iter()
                .enumerate()
                .skip(first_close + 1)
                .take_while(|(_, character)| **character != '］')
                .find_map(|(index, character)| (*character == '」').then_some(index))
                .unwrap_or(first_close)
        }
    };
    let close = chars
        .iter()
        .enumerate()
        .skip(target_end + 1)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let target = chars[target_start + 1..target_end]
        .iter()
        .collect::<String>();
    // Java chukiSufPattern の suffix は [^」|^］]+（先頭の 」 を含まない）
    let suffix = chars[target_end + 1..close]
        .iter()
        .collect::<String>()
        .trim_start_matches('」')
        .to_owned();
    (!target.is_empty() && !suffix.is_empty()).then_some((close + 1, target, suffix))
}

fn suffix_visible_text(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '［' && chars.get(index + 1) == Some(&'＃') {
            index = chars
                .iter()
                .enumerate()
                .skip(index + 2)
                .find_map(|(candidate, value)| (*value == '］').then_some(candidate + 1))
                .unwrap_or(index + 1);
            continue;
        }
        if chars[index] == '｜' {
            index += 1;
            continue;
        }
        if chars[index] == '《' {
            index = chars
                .iter()
                .enumerate()
                .skip(index + 1)
                .find_map(|(candidate, value)| (*value == '》').then_some(candidate + 1))
                .unwrap_or(index + 1);
            continue;
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}



fn suffix_target_range_by_len(prefix: &str, target_len: usize) -> Option<(usize, usize)> {
    if target_len == 0 {
        return None;
    }
    // Java getTargetStart: 注記の直前から可視文字を target_len 分だけ遡る。
    // 間にあるルビ（《…》）と注記タグ（［＃…］）は除外、｜は数えない。
    let indexed = prefix.char_indices().collect::<Vec<_>>();
    let mut idx = indexed.len();
    let mut length = 0usize;
    let mut has_ruby = false;
    while target_len > length && idx > 0 {
        match indexed[idx - 1].1 {
            '》' => {
                let mut j = idx - 1;
                while j > 0 && indexed[j - 1].1 != '《' {
                    j -= 1;
                }
                idx = j.saturating_sub(1);
                has_ruby = true;
                continue;
            }
            '］' => {
                let mut j = idx - 1;
                while j > 0 && indexed[j - 1].1 != '［' {
                    j -= 1;
                }
                idx = j.saturating_sub(1);
                continue;
            }
            '｜' => {}
            _ => length += 1,
        }
        idx -= 1;
    }
    let mut start = indexed[idx].0;
    // ルビをまたいだら先頭の｜を含める
    if has_ruby && start >= '｜'.len_utf8() && prefix[..start].ends_with('｜') {
        start -= '｜'.len_utf8();
    }
    Some((start, prefix.len()))
}

fn suffix_target_range(prefix: &str, target: &str) -> Option<(usize, usize)> {
    suffix_target_range_by_len(prefix, suffix_visible_text(target).chars().count())
}

fn parse_unicode_note(
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
    let replacement = unicode_replacement(&note, config)?;
    Some((close + 1, replacement))
}

fn unicode_replacement(note: &str, config: &AozoraConfig) -> Option<String> {
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
    Some(render_gaiji_replacement(&replacement, config, true))
}

fn render_gaiji_replacement(input: &str, config: &AozoraConfig, allow_upright: bool) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < chars.len() {
        if let Some((class_name, consumed)) = glyph_font_for_sequence(&chars, index, config) {
            output.push_str(&glyph_span(&class_name, '〓'));
            index += consumed;
            continue;
        }
        if is_bmp_variation(chars[index]) && !config.print_ivs_bmp
            || is_ssp_variation(chars[index]) && !config.print_ivs_ssp
        {
            index += 1;
            continue;
        }
        index += push_text_char(&mut output, &chars, index, config, allow_upright);
    }
    output
}

fn glyph_font_for_sequence(
    chars: &[char],
    index: usize,
    config: &AozoraConfig,
) -> Option<(String, usize)> {
    let base = *chars.get(index)?;
    if let Some(variation) = chars.get(index + 1).copied().and_then(variation_code) {
        let ivs_class = format!("u{:x}-u{:x}", base as u32, variation);
        if config.gaiji_font(&ivs_class).is_some() {
            return Some((ivs_class, 2));
        }
        let base_class = format!("u{:x}", base as u32);
        if config.gaiji_font(&base_class).is_some() {
            return Some((base_class, 2));
        }
    }
    let base_class = format!("u{:x}", base as u32);
    config.gaiji_font(&base_class).map(|_| (base_class, 1))
}

fn variation_code(character: char) -> Option<u32> {
    is_bmp_variation(character)
        .then_some(character as u32)
        .or_else(|| is_ssp_variation(character).then_some(character as u32))
}

fn is_bmp_variation(character: char) -> bool {
    ('\u{fe00}'..='\u{fe0f}').contains(&character)
}

fn is_ssp_variation(character: char) -> bool {
    ('\u{e0100}'..='\u{e01ef}').contains(&character)
}

fn glyph_span(class_name: &str, base: char) -> String {
    format!(
        "<span class=\"glyph {class_name}\">{}</span>",
        escape_text(&base.to_string())
    )
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
fn parse_gaiji_note(
    chars: &[char],
    start: usize,
    config: &AozoraConfig,
    allow_upright: bool,
) -> Option<(usize, String)> {
    let (end, note) = gaiji_note_range(chars, start)?;
    let bare_note = note
        .strip_prefix("※［＃")
        .and_then(|value| value.strip_suffix('］'))
        .unwrap_or(&note);
    let key = bare_note.split(['、', ',']).next().unwrap_or(bare_note);
    let normalized_key = crate::config::normalize_gaiji_key(key);
    let key_note = format!("※［＃{key}］");
    let normalized_key_note = format!("※［＃{normalized_key}］");
    if bare_note.contains("※［＃") {
        // Java: 内側注記を含む外字注記は説明（行右小書き）に変換される
        let key = bare_note.split(['、', ',']).next().unwrap_or(bare_note);
        let open = config
            .inline_notes
            .get("行右小書き")
            .map(String::as_str)
            .unwrap_or("<span class=\"super\">");
        let close = config
            .inline_notes
            .get("行右小書き終わり")
            .map(String::as_str)
            .unwrap_or("</span>");
        return Some((end, format!("〓{open}（{key}）{close}")));
    }
    if let Some(replacement) = unicode_replacement(bare_note, config) {
        return Some((end, replacement));
    }
    let normalized_note = crate::config::normalize_gaiji_key(&note);
    if let Some(replacement) = unicode_replacement_in_following_text(chars, end, config) {
        return Some((end, replacement));
    }
    if let Some(replacement) = config
        .gaiji
        .get(&note)
        .or_else(|| config.gaiji.get(&normalized_note))
        .or_else(|| config.gaiji.get(bare_note))
        .or_else(|| config.gaiji.get(key))
        .or_else(|| config.gaiji.get(&normalized_key))
        .or_else(|| config.gaiji.get(&key_note))
        .or_else(|| config.gaiji.get(&normalized_key_note))
    {
        return Some((end, render_gaiji_replacement(replacement, config, allow_upright)));
    }
    if let Some(replacement) = jis_note_replacement(bare_note, config, allow_upright) {
        return Some((end, replacement));
    }
    let open = config
        .inline_notes
        .get("行右小書き")
        .map(String::as_str)
        .unwrap_or("<span class=\"super\">");
    let close = config
        .inline_notes
        .get("行右小書き終わり")
        .map(String::as_str)
        .unwrap_or("</span>");
    let description = convert_inline(key, config);
    Some((end, format!("〓{open}（{description}）{close}")))
}

/// Resolves JIS X 0213 丸数字 codes (1面 8/12/13区) used by the reference
/// converter's JisConverter table. Only the JIS code part (after the last
/// `、` or after the 第3/第4水準 marker) is parsed.
fn jis_note_replacement(
    note: &str,
    config: &AozoraConfig,
    allow_upright: bool,
) -> Option<String> {
    let code_part = if let Some(position) = note.find("第3水準") {
        &note[position + "第3水準".len()..]
    } else if let Some(position) = note.find("第4水準") {
        &note[position + "第4水準".len()..]
    } else {
        let comma = note.rfind('、')?;
        &note[comma + '、'.len_utf8()..]
    };
    let start = code_part.find(|c: char| c.is_ascii_digit())?;
    let mut parts = code_part[start..].split(|c: char| !c.is_ascii_digit());
    let plane = parts.next()?.parse::<u8>().ok()?;
    let row = parts.next()?.parse::<u8>().ok()?;
    let cell = parts.next()?.parse::<u8>().ok()?;
    let character = jis_to_unicode(plane, row, cell)?;
    Some(render_gaiji_replacement(&character.to_string(), config, allow_upright))
}

/// JIS X 0213 1面 8区(㉑-㊿)・12区(❶-❿,⓫-⓴)・13区(①-⑳) → Unicode.
fn jis_to_unicode(plane: u8, row: u8, cell: u8) -> Option<char> {
    if plane != 1 {
        return None;
    }
    let code = match row {
        8 if (33..=47).contains(&cell) => 0x3251 + (cell - 33) as u32,
        8 if (48..=62).contains(&cell) => 0x32b1 + (cell - 48) as u32,
        12 if (1..=10).contains(&cell) => 0x2776 + (cell - 1) as u32,
        12 if (11..=20).contains(&cell) => 0x24eb + (cell - 11) as u32,
        13 if (1..=20).contains(&cell) => 0x2460 + (cell - 1) as u32,
        _ => return None,
    };
    char::from_u32(code)
}

fn gaiji_note_range(chars: &[char], start: usize) -> Option<(usize, String)> {
    if chars.get(start) != Some(&'※')
        || chars.get(start + 1) != Some(&'［')
        || chars.get(start + 2) != Some(&'＃')
    {
        return None;
    }
    // Java chukiPattern は最初の ］ まででマッチする（注記内注記は注記文字変換で除去）
    let close = chars
        .iter()
        .enumerate()
        .skip(start + 3)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let note = chars[start..=close].iter().collect::<String>();
    Some((close + 1, note))
}

fn unicode_replacement_in_following_text(
    chars: &[char],
    start: usize,
    config: &AozoraConfig,
) -> Option<String> {
    let tail = chars[start..]
        .iter()
        .take_while(|character| **character != '\n' && **character != '\r')
        .collect::<String>();
    unicode_replacement(&tail, config)
}

fn parse_image_note(
    chars: &[char],
    start: usize,
    config: &AozoraConfig,
) -> Option<(usize, String)> {
    let (end, path, description, is_gaiji) = image_note_parts(chars, start)?;
    let source = format!("../image/{}", escape_html(&path));
    if is_gaiji {
        let replacement = config
            .inline_notes
            .get("外字画像")
            .map(|template| format_image_template(template, &source, ""))
            .unwrap_or_else(|| format!(r#"<img class="gaiji" src="{source}" alt=""/>"#));
        return Some((end, replacement));
    }
    let alt = if let Some(stem) = image_stem(&path)
        && let Some(mapped) = config.image_alt_map.get(&stem)
    {
        escape_html(mapped).replace('×', "&times;")
    } else if config.inline_notes.contains_key("画像") && !description.is_empty() {
        escape_html(&description).replace('×', "&times;")
    } else {
        String::new()
    };

    let mut replacement = config
        .inline_notes
        .get("画像")
        .map(|template| format_image_template(template, &source, &alt))
        .unwrap_or_else(|| format!("<img class=\"fit\" src=\"{source}\" alt=\"{alt}\"/>"));
    if !description.contains("キャプション付き")
        && let Some(close) = config.inline_notes.get("画像終わり")
    {
        replacement.push_str(close);
    }
    Some((end, replacement))
}

fn format_image_template(template: &str, source: &str, alt: &str) -> String {
    let mut parts = template.split("%s");
    let mut output = parts.next().unwrap_or_default().to_owned();
    if let Some(part) = parts.next() {
        output.push_str(source);
        output.push_str(part);
    }
    if let Some(part) = parts.next() {
        output.push_str(alt);
        output.push_str(part);
    }
    output
}

fn parse_inline_heading(
    chars: &[char],
    start: usize,
    config: &AozoraConfig,
) -> Option<(usize, String)> {
    let (note_end, note) = note_range(chars, start)?;
    let spec = heading_spec(&note)?;
    let close_note = format!("{note}終わり");
    let (close_start, close_end) = find_note(chars, note_end, &close_note)?;
    let inner = chars[note_end..close_start].iter().collect::<String>();
    let mut replacement = format!("<{} class=\"{}\">", spec.element, spec.class_name);
    replacement.push_str(&convert_inline(&inner, config));
    replacement.push_str("</");
    replacement.push_str(spec.element);
    replacement.push('>');
    Some((close_end, replacement))
}

fn parse_configured_inline_block(
    chars: &[char],
    start: usize,
    config: &AozoraConfig,
) -> Option<(usize, String)> {
    let (note_end, note) = note_range(chars, start)?;
    let (open_tag, close_tag) = config.block_inline_tags.get(&note)?;
    let close_note = format!("{note}終わり");
    let (close_start, close_end) = find_note(chars, note_end, &close_note)?;
    let inner = chars[note_end..close_start].iter().collect::<String>();
    let mut replacement = open_tag.clone();
    replacement.push_str(&convert_inline(&inner, config));
    replacement.push_str(close_tag);
    Some((close_end, replacement))
}

fn parse_raw_image(chars: &[char], start: usize, config: &AozoraConfig) -> Option<(usize, String)> {
    let end = chars
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, character)| (*character == '>').then_some(index))?;
    let raw = chars[start..=end].iter().collect::<String>();
    let lower = raw.to_ascii_lowercase();
    if !lower.starts_with("<img") {
        return None;
    }
    let source = raw_tag_attribute(&raw, "src")?;
    if source.trim().is_empty() {
        // Java: src 空の img は画像取得失敗で出力されない
        return Some((end + 1, String::new()));
    }
    let source = normalize_image_path(source.trim())?;
    let alt = escape_html(raw_tag_attribute(&raw, "alt").unwrap_or_default().trim());
    let source = format!("../image/{}", escape_html(&source));
    let replacement = config
        .inline_notes
        .get("画像")
        .map(|template| format_image_template(template, &source, &alt))
        .unwrap_or_else(|| format!("<img class=\"fit\" src=\"{source}\" alt=\"{alt}\"/>"));
    let replacement = if replacement.contains("<img") && !replacement.contains("</span>") {
        if let Some(close) = config.inline_notes.get("画像終わり") {
            format!("{replacement}{close}")
        } else {
            replacement
        }
    } else {
        replacement
    };
    Some((end + 1, replacement))
}
fn raw_tag_attribute<'a>(tag: &'a str, attribute: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let marker = format!("{attribute}=");
    let start = lower.match_indices(&marker).find_map(|(index, _)| {
        let before = lower[..index].chars().next_back();
        (before.is_none() || before.is_some_and(|character| character.is_ascii_whitespace()))
            .then_some(index + marker.len())
    })?;
    let quote = tag[start..].chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let value = tag[start..].strip_prefix(quote)?;
    let end = value.find(quote)?;
    Some(&value[..end])
}


fn parse_raw_anchor(chars: &[char], start: usize) -> Option<(usize, String)> {
    let end = chars
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, character)| (*character == '>').then_some(index))?;
    let raw = chars[start..=end].iter().collect::<String>();
    let lower = raw.to_ascii_lowercase();
    let replacement = if lower == "</a>" {
        "</a>".to_owned()
    } else if lower.starts_with("<a") {
        // Java: href が http または # で始まる場合のみタグを出力し、
        // それ以外（name のみ・その他 href）は破棄する
        if let Some(href) = raw_tag_attribute(&raw, "href") {
            if href.contains('"') || href.contains('<') || href.contains('>') {
                return None;
            }
            let href = href.trim();
            if href.starts_with("http") || href.starts_with('#') {
                raw.replace('&', "&amp;")
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    } else {
        return None;
    };
    Some((end + 1, replacement))
}

fn note_range(chars: &[char], start: usize) -> Option<(usize, String)> {
    if chars.get(start) != Some(&'［') || chars.get(start + 1) != Some(&'＃') {
        return None;
    }
    let close = chars
        .iter()
        .enumerate()
        .skip(start + 2)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let note = chars[start + 2..close].iter().collect::<String>();
    Some((close + 1, note))
}
fn gaiji_note_start_ending_at(chars: &[char], end: usize) -> Option<usize> {
    if end == 0 || chars.get(end - 1) != Some(&'］') {
        return None;
    }
    (0..end).rev().find_map(|start| {
        if start == 0
            || chars.get(start) != Some(&'［')
            || chars.get(start + 1) != Some(&'＃')
            || chars.get(start - 1) != Some(&'※')
        {
            return None;
        }
        (note_range(chars, start).is_some_and(|(note_end, _)| note_end == end)).then_some(start - 1)
    })
}

fn image_note_start_ending_at(chars: &[char], end: usize) -> Option<usize> {
    if end == 0 || chars.get(end - 1) != Some(&'］') {
        return None;
    }
    (0..end).rev().find_map(|start| {
        let (note_end, _) = note_range(chars, start)?;
        (note_end == end && image_note_parts(chars, start).is_some()).then_some(start)
    })
}

fn unicode_note_start_ending_at(
    chars: &[char],
    end: usize,
    config: &AozoraConfig,
) -> Option<usize> {
    if end == 0 || chars.get(end - 1) != Some(&'］') {
        return None;
    }
    (0..end).rev().find_map(|start| {
        if start == 0
            || chars.get(start) != Some(&'［')
            || chars.get(start + 1) != Some(&'＃')
            || chars.get(start - 1) != Some(&'※')
        {
            return None;
        }
        (parse_unicode_note(chars, start, config).is_some_and(|(note_end, _)| note_end == end))
            .then_some(start - 1)
    })
}

/// 注記を本文と同じ経路で描画し、全文字が同一の基底種別ならその種別を返す。
/// ルビ基底に注記を含めてよいかの判定に使う（縦線→｜ は基底にならない）。
fn note_rendered_kind(note: &[char], config: &AozoraConfig) -> Option<u8> {
    let mut rendered = String::new();
    let mut index = 0;
    while index < note.len() {
        if let Some((end, replacement)) = parse_image_note(note, index, config) {
            rendered.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_gaiji_note(note, index, config, true) {
            rendered.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_unicode_note(note, index, config) {
            rendered.push_str(&replacement);
            index = end;
            continue;
        }
        if let Some((end, replacement)) = parse_inline_note(note, index, config) {
            rendered.push_str(&replacement);
            index = end;
            continue;
        }
        break;
    }
    if rendered.is_empty() {
        // IVS等、出力が空になる注記は基底を継続させる（空描画）
        return Some(EMPTY_NOTE_KIND);
    }
    let mut kinds = rendered.chars().map(ruby_base_kind);
    let first = kinds.next()??;
    kinds.all(|kind| kind == Some(first)).then_some(first)
}
fn latin_bracket_start_ending_at(chars: &[char], end: usize) -> Option<usize> {
    if end == 0 || chars.get(end - 1) != Some(&'〕') {
        return None;
    }
    (0..end).rev().find(|index| chars[*index] == '〔')
}

fn find_note(chars: &[char], start: usize, note: &str) -> Option<(usize, usize)> {
    let marker = format!("［＃{note}］").chars().collect::<Vec<_>>();
    if marker.is_empty() || start >= chars.len() {
        return None;
    }
    (start..=chars.len().saturating_sub(marker.len())).find_map(|index| {
        (chars.get(index..index + marker.len()) == Some(marker.as_slice()))
            .then_some((index, index + marker.len()))
    })
}

fn image_note_parts(chars: &[char], start: usize) -> Option<(usize, String, String, bool)> {
    if chars.get(start) != Some(&'［') || chars.get(start + 1) != Some(&'＃') {
        return None;
    }
    let close = chars
        .iter()
        .enumerate()
        .skip(start + 2)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let note = chars[start + 2..close].iter().collect::<String>();
    let open_paren = note.find('（')?;
    let close_paren = note.rfind('）')?;
    if open_paren >= close_paren {
        return None;
    }
    let inside = &note[open_paren + '（'.len_utf8()..close_paren];
    let path = inside.split('、').next()?.trim();
    if !path.contains('.') {
        return None;
    }
    let path = normalize_image_path(path)?;
    let description = note[..open_paren]
        .trim()
        .strip_suffix("入る")
        .unwrap_or(note[..open_paren].trim())
        .to_owned();
    Some((close + 1, path, description, note.contains("#GAIJI#")))
}

fn image_path_from_note(chars: &[char], start: usize) -> Option<(usize, String)> {
    let (end, path, _, _) = image_note_parts(chars, start)?;
    Some((end, path))
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

pub fn image_reference_occurrences(input: &str) -> Vec<String> {
    let chars = input.chars().collect::<Vec<_>>();
    let byte_offsets = input
        .char_indices()
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();

    let mut index = 0;
    while index < chars.len() {
        if let Some((end, path)) = image_path_from_note(&chars, index) {
            candidates.push((byte_offsets[index], path));
            index = end;
        } else {
            index += 1;
        }
    }

    let mut index = 0;
    while index + 3 < chars.len() {
        if chars[index] != '<'
            || !chars[index + 1].eq_ignore_ascii_case(&'i')
            || !chars[index + 2].eq_ignore_ascii_case(&'m')
            || !chars[index + 3].eq_ignore_ascii_case(&'g')
        {
            index += 1;
            continue;
        }
        let Some(end_offset) = chars[index..]
            .iter()
            .position(|character| *character == '>')
        else {
            break;
        };
        let end = index + end_offset + 1;
        let raw = chars[index..end].iter().collect::<String>();
        if let Some(source) =
            raw_tag_attribute(&raw, "src").and_then(|source| normalize_image_path(source.trim()))
        {
            candidates.push((byte_offsets[index], source));
        }
        index = end;
    }

    candidates.sort_by_key(|(offset, _)| *offset);
    candidates.into_iter().map(|(_, path)| path).collect()
}

pub fn image_references(input: &str) -> Vec<String> {
    let mut references = Vec::new();
    for path in image_reference_occurrences(input) {
        if !references.contains(&path) {
            references.push(path);
        }
    }
    references
}

/// 本文全体から画像の alt を収集する（Java の imageAltMap 相当）。
/// 画像注記の説明文と raw `<img>` の alt をファイル名ステムで最後勝ちで記録する。
pub fn collect_image_alts(input: &str, config: &mut AozoraConfig) {
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if let Some((end, path, description, _)) = image_note_parts(&chars, index) {
            let description = description.trim();
            if !description.is_empty()
                && let Some(stem) = image_stem(&path)
            {
                config
                    .image_alt_map
                    .insert(stem, description.to_owned());
            }
            index = end;
            continue;
        }
        if chars[index] == '<'
            && chars.get(index + 1).is_some_and(|c| c.eq_ignore_ascii_case(&'i'))
            && chars.get(index + 2).is_some_and(|c| c.eq_ignore_ascii_case(&'m'))
            && chars.get(index + 3).is_some_and(|c| c.eq_ignore_ascii_case(&'g'))
        {
            let Some(end_offset) = chars[index..]
                .iter()
                .position(|character| *character == '>')
            else {
                break;
            };
            let end = index + end_offset + 1;
            let raw = chars[index..end].iter().collect::<String>();
            let alt = raw_tag_attribute(&raw, "alt").unwrap_or_default().trim();
            if !alt.is_empty()
                && let Some(source) =
                    raw_tag_attribute(&raw, "src").and_then(|source| normalize_image_path(source.trim()))
                && let Some(stem) = image_stem(&source)
            {
                config.image_alt_map.insert(stem, alt.to_owned());
            }
            index = end;
            continue;
        }
        index += 1;
    }
}

fn image_stem(path: &str) -> Option<String> {
    path.rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(path)
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .filter(|stem| !stem.is_empty())
        .map(str::to_owned)
}

fn parse_inline_note(
    chars: &[char],
    start: usize,
    config: &AozoraConfig,
) -> Option<(usize, String)> {
    if chars.get(start) != Some(&'［') || chars.get(start + 1) != Some(&'＃') {
        return None;
    }
    // 注記内注記（［＃…［＃…］…］）を考慮して閉じ括弧を探す
    let mut depth = 1usize;
    let mut index = start + 2;
    let close = loop {
        if chars.get(index) == Some(&'［') && chars.get(index + 1) == Some(&'＃') {
            depth += 1;
            index += 2;
            continue;
        }
        if chars.get(index) == Some(&'］') {
            depth -= 1;
            if depth == 0 {
                break index;
            }
        }
        index += 1;
        if index >= chars.len() {
            return None;
        }
    };
    let note = chars[start + 2..close].iter().collect::<String>();
    // 訓点送り仮名・返り点: ［＃（X）］ → 行右小書き（. を含む外字画像は除外）
    if let Some(inner) = note.strip_prefix('（').and_then(|value| value.strip_suffix('）'))
        && !note.contains('.')
        && !inner.is_empty()
    {
        let open = config
            .inline_notes
            .get("行右小書き")
            .map(String::as_str)
            .unwrap_or("<span class=\"super\">");
        let close_tag = config
            .inline_notes
            .get("行右小書き終わり")
            .map(String::as_str)
            .unwrap_or("</span>");
        return Some((close + 1, format!("{open}{inner}{close_tag}")));
    }
    let replacement = config
        .inline_notes
        .get(&note)
        .cloned()
        .or_else(|| should_preserve_unconverted_note(&note).then(|| format!("［＃{note}］")))
        .unwrap_or_default();
    Some((close + 1, replacement))
}
fn should_preserve_unconverted_note(note: &str) -> bool {
    note.contains('「')
        && note.contains('」')
        && !note.contains("左に")
        && !note.contains("底本では")
        && !note.contains("ママ")
        // Java: 小書き対応外の「…」に「…」の注記/ルビは破棄される
        && !note.contains("」に「")
        && !note.ends_with("の注記")
}
fn parse_configured_markup(
    chars: &[char],
    start: usize,
    config: &AozoraConfig,
) -> Option<(usize, String)> {
    if chars.get(start) != Some(&'<') {
        return None;
    }
    let close = chars
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, character)| (*character == '>').then_some(index))?;
    let markup = chars[start..=close].iter().collect::<String>();
    config
        .inline_notes
        .values()
        .any(|value| value == &markup)
        .then_some((close + 1, markup))
}

fn is_tcy_open(markup: &str, config: &AozoraConfig) -> bool {
    config.inline_notes.get("縦中横").map(String::as_str) == Some(markup)
}

fn is_tcy_close(markup: &str, config: &AozoraConfig) -> bool {
    config.inline_notes.get("縦中横終わり").map(String::as_str) == Some(markup)
}

fn push_text_char(
    output: &mut String,
    chars: &[char],
    index: usize,
    config: &AozoraConfig,
    allow_upright: bool,
) -> usize {
    // Single-character replacement (replace.txt): the result is emitted raw,
    // exactly like the reference replaceMap, so it is not re-normalized.
    let key = chars[index].to_string();
    if let Some(replacement) = config.character_replacements.get(&key) {
        output.push_str(replacement);
        return 1;
    }
    if let Some((class_name, consumed)) = glyph_font_for_sequence(chars, index, config) {
        output.push_str(&glyph_span(&class_name, '〓'));
        return consumed;
    }
    let next_mark = chars
        .get(index + 1)
        .copied()
        .and_then(normalize_dakuten_mark);
    if config.vertical
        && let Some(mark) = next_mark
        && is_dakuten_base(chars[index])
    {
        // Java composes the standard kana pairs before applying the
        // configured fallback for otherwise unsupported combinations.
        if let Some(composed) = compose_dakuten(chars[index], mark) {
            push_text_char_escaped(output, composed);
            return 2;
        }
        if config.dakuten_type == 2
            && config
                .gaiji_font(&format!(
                    "u{:x}-u{:x}",
                    chars[index] as u32,
                    if mark == '゛' { 0x3099 } else { 0x309a }
                ))
                .is_some()
        {
            let class_name = format!(
                "u{:x}-u{:x}",
                chars[index] as u32,
                if mark == '゛' { 0x3099 } else { 0x309a }
            );
            output.push_str(&glyph_span(&class_name, chars[index]));
            return 2;
        }
        if config.dakuten_type == 1 {
            output.push_str("<span class=\"dakuten\">");
            push_text_char_escaped(output, chars[index]);
            output.push_str("<span>");
            push_text_char_escaped(output, mark);
            output.push_str("</span></span>");
            return 2;
        }
    }

    let character = normalize_vertical_character(chars[index], config.vertical);
    if allow_upright && is_upright_character(character) {
        output.push_str("<span class=\"upr\">");
        push_text_char_escaped(output, character);
        output.push_str("</span>");
    } else {
        push_text_char_escaped(output, character);
    }
    1
}

fn normalize_dakuten_mark(character: char) -> Option<char> {
    match character {
        '゛' | '゙' => Some('゛'),
        '゜' | '゚' => Some('゜'),
        _ => None,
    }
}

fn normalize_vertical_character(character: char, vertical: bool) -> char {
    match (vertical, character) {
        (true, '≪') | (false, '≪') => '《',
        (true, '≫') | (false, '≫') => '》',
        (true, '“') => '〝',
        (true, '”') => '〟',
        (_, '―') => '─',
        (_, '゙') => '゛',
        (_, '゚') => '゜',
        _ => character,
    }
}

fn is_dakuten_base(character: char) -> bool {
    matches!(
        character,
        'ぁ'..='ゖ'
            | 'ゝ'
            | 'ァ'..='ヺ'
            | 'ヽ'
            | 'ヿ'
            | '〻'
            | 'ー'
            | 'ι'
            | '\u{31f0}'..='\u{31ff}'
    )
}

fn compose_dakuten(base: char, mark: char) -> Option<char> {
    let (from, to) = if mark == '゛' {
        (
            "うかきくけこさしすせそたちつてとはひふへほゝウカキクケコサシスセソタチツテトハヒフヘホワヰヱヲヽ",
            "ゔがぎぐげござじずぜぞだぢづでどばびぶべぼゞヴガギグゲゴザジズゼゾダヂヅデドバビブベボヷヸヹヺヾ",
        )
    } else {
        ("はひふへほハヒフヘホ", "ぱぴぷぺぽパピプペポ")
    };
    from.chars()
        .position(|candidate| candidate == base)
        .and_then(|position| to.chars().nth(position))
}

fn is_upright_character(character: char) -> bool {
    const UPRIGHT: &str = "÷±∞∴∵ⅠⅡⅢⅣⅤⅥⅦⅧⅨⅩⅪⅫ\
ⅰⅱⅲⅳⅴⅵⅶⅷⅸⅹⅺⅻ\
⓪①②③④⑤⑥⑦⑧⑨⑩⑪⑫⑬⑭⑮⑯⑰⑱⑲⑳\
㉑㉒㉓㉔㉕㉖㉗㉘㉙㉚㉛㉜㉝㉞㉟㊱㊲㊳㊴㊵㊶㊷㊸㊹㊺㊻㊼㊽㊾㊿\
△▽▲▼☆★♂♀♪♭§†‡‼⁇⁉⁈©®⁑⁂◐◑◒◓▷▶◁◀\
♤♠♢♦♡♥♧♣❤☖☗☎☁☂☃♨▱⊿✿☹☺☻✓✔␣⏎♩♮♫♬ℓ№℡ℵℏ℧";
    UPRIGHT.contains(character)
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
fn ruby_base_kind(character: char) -> Option<u8> {
    match character as u32 {
        0x3005..=0x3006
        | 0x3400..=0x4dbf
        | 0x4e00..=0x9fff
        | 0xf900..=0xfaff
        | 0x20000..=0x2ffff => Some(0),
        0x3041..=0x3096
        | 0x309b..=0x309e
        | 0x30fc
        | 0x30fd..=0x30fe => Some(1),
        0x30a1..=0x30fa | 0xff61..=0xff9f => Some(2),
        0x20..=0x7e | 0xa0..=0x2af => Some(3),
        0xff10..=0xff19 | 0xff21..=0xff3a | 0xff41..=0xff5a => Some(4),
        _ => None,
    }
}

fn image_note_followed_by_ruby(chars: &[char], index: usize) -> bool {
    let Some((note_end, _, _, _)) = image_note_parts(chars, index) else {
        return false;
    };
    chars.get(note_end) == Some(&'《') && find_closing_ruby(chars, note_end).is_some()
}

fn has_following_implicit_ruby(chars: &[char], mut index: usize) -> bool {
    if image_note_followed_by_ruby(chars, index) {
        return true;
    }
    let Some(first_kind) = chars
        .get(index)
        .and_then(|character| ruby_base_kind(*character))
    else {
        return false;
    };
    while let Some(&character) = chars.get(index) {
        if character == '《' {
            return index > 0
                && find_closing_ruby(chars, index).is_some()
                && ruby_base_kind(chars[index - 1]) == Some(first_kind);
        }
        if ruby_base_kind(character) != Some(first_kind) {
            return false;
        }
        index += 1;
    }
    false
}

fn push_ruby(
    output: &mut String,
    base: &str,
    reading: &str,
    config: &AozoraConfig,
    allow_auto_yoko: bool,
) {
    if output.ends_with("</ruby>") {
        output.truncate(output.len() - "</ruby>".len());
    } else {
        output.push_str("<ruby>");
    }
    push_ruby_part(output, base, reading, config, allow_auto_yoko);
    output.push_str("</ruby>");
}

fn push_ruby_part(
    output: &mut String,
    base: &str,
    reading: &str,
    config: &AozoraConfig,
    allow_auto_yoko: bool,
) {
    let base_chars = base.chars().collect::<Vec<_>>();
    let reading_chars = reading.chars().collect::<Vec<_>>();
    // Java: 基底と読仮名が同じ長さで読仮名が同一文字なら一文字ずつルビを振る
    if base_chars.len() == reading_chars.len()
        && base_chars.len() > 1
        && reading_chars.iter().all(|character| *character == reading_chars[0])
    {
        for (base_char, reading_char) in base_chars.iter().zip(reading_chars.iter()) {
            if allow_auto_yoko {
                output.push_str(&convert_inline(&base_char.to_string(), config));
            } else {
                output.push_str(&convert_inline_without_auto_yoko(&base_char.to_string(), config));
            }
            output.push_str("<rt>");
            output.push_str(&convert_ruby_reading(&reading_char.to_string(), config));
            output.push_str("</rt>");
        }
        return;
    }
    if allow_auto_yoko && !contains_literal_gaiji_note(base) {
        output.push_str(&convert_inline(base, config));
    } else {
        output.push_str(&convert_inline_without_auto_yoko(base, config));
    }
    output.push_str("<rt>");
    output.push_str(&convert_ruby_reading(reading, config));
    output.push_str("</rt>");
}

/// Escapes text for attribute values (quotes included).
pub fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

/// Escapes body text the way the reference converter does: only `& < >`.
fn escape_text(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for character in input.chars() {
        push_text_char_escaped(&mut escaped, character);
    }
    escaped
}

fn push_text_char_escaped(output: &mut String, character: char) {
    match character {
        '&' => output.push_str("&amp;"),
        '<' => output.push_str("&lt;"),
        '>' => output.push_str("&gt;"),
        _ => output.push(character),
    }
}






