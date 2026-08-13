use super::{AozoraConfig, heading_spec};
const WRC_BREAK_MARKER: char = '\u{0001}';

pub(super) fn convert_inline(input: &str, config: &AozoraConfig) -> String {
    convert_inline_with_auto_yoko(input, config, true)
}

fn convert_inline_without_auto_yoko(input: &str, config: &AozoraConfig) -> String {
    convert_inline_with_auto_yoko(input, config, false)
}

fn convert_inline_with_auto_yoko(input: &str, config: &AozoraConfig, auto_yoko: bool) -> String {
    let input = rewrite_character_replacements(input, config);
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
    while index < chars.len() {
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
        if chars[index] == '※'
            && let Some((end, replacement)) = parse_image_note(&chars, index + 1, config)
        {
            output.push('※');
            output.push_str(&replacement);
            index = end;
            continue;
        }
        if chars[index] == '※'
            && let Some((end, replacement)) = parse_gaiji_note(&chars, index, config)
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
                push_ruby(
                    &mut output,
                    &base,
                    &reading,
                    config,
                    auto_yoko && tcy_depth == 0,
                );
                index = close + 1;
                continue;
            }
        }

        if chars[index] == '《'
            && let Some(close) = find_closing_ruby(&chars, index)
            && index > 0
            && ruby_base_kind(chars[index - 1]).is_some()
        {
            let mut base_start = index - 1;
            while base_start > 0 {
                let Some(current_kind) = ruby_base_kind(chars[base_start]) else {
                    break;
                };
                let Some(previous_kind) = ruby_base_kind(chars[base_start - 1]) else {
                    break;
                };
                if previous_kind != current_kind {
                    break;
                }
                base_start -= 1;
            }
            let base = chars[base_start..index].iter().collect::<String>();
            let escaped_base = escape_html(&base);
            if output.ends_with(&escaped_base) {
                output.truncate(output.len() - escaped_base.len());
                let reading = chars[index + 1..close].iter().collect::<String>();
                push_ruby(
                    &mut output,
                    &base,
                    &reading,
                    config,
                    auto_yoko && tcy_depth == 0,
                );
                index = close + 1;
                continue;
            }
        }

        index += push_text_char(&mut output, &chars, index, config, tcy_depth == 0);
    }
    output
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
        let replacement = [2, 1].into_iter().find_map(|length| {
            let candidate = chars.get(index..index + length)?;
            let key = candidate.iter().collect::<String>();
            config
                .character_replacements
                .get(&key)
                .map(|value| (length, value.as_str()))
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
        let target_at_end = target_end == prefix.len();
        let start_note = format!("［＃{start_tag}］");
        let end_note = format!("［＃{end_tag}］");
        let mut rewritten = prefix;
        rewritten.insert_str(target_start, &start_note);

        // A suffix note may appear between an explicit ruby base and its
        // reading marker. Keep the generated span outside the complete ruby.
        let ruby_end = suffix.strip_prefix('《').and_then(|reading| {
            reading
                .find('》')
                .map(|offset| '《'.len_utf8() + offset + '》'.len_utf8())
        });
        if target_at_end && let Some(ruby_end) = ruby_end {
            rewritten.push_str(&suffix[..ruby_end]);
            rewritten.push_str(&end_note);
            rewritten.push_str(&suffix[ruby_end..]);
        } else {
            rewritten.insert_str(target_end + start_note.len(), &end_note);
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
            let visible_target = suffix_visible_text(&target);
            let (mut target_start, target_end) = suffix_target_range(&prefix, &visible_target)?;
            if target.contains('《') {
                let extra = target
                    .chars()
                    .count()
                    .saturating_sub(visible_target.chars().count());
                for _ in 0..extra {
                    target_start = previous_char_boundary(&prefix, target_start);
                }
                while target_start > 0 && prefix[..target_start].ends_with('｜') {
                    target_start -= '｜'.len_utf8();
                }
            }
            let mut rewritten = String::with_capacity(prefix.len() + reading.len() + 4);
            rewritten.push_str(&prefix[..target_start]);
            rewritten.push('｜');
            if target.contains('《') {
                rewritten.push_str(&remove_suffix_ruby(&prefix[target_start..target_end]));
            } else {
                rewritten.push_str(&prefix[target_start..target_end]);
            }
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

fn convert_ruby_reading(reading: &str, _config: &AozoraConfig) -> String {
    escape_html(reading)
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
    let target_end = chars
        .iter()
        .enumerate()
        .skip(target_start + 1)
        .find_map(|(index, character)| (*character == '」').then_some(index))?;
    let close = chars
        .iter()
        .enumerate()
        .skip(target_end + 1)
        .find_map(|(index, character)| (*character == '］').then_some(index))?;
    let target = chars[target_start + 1..target_end]
        .iter()
        .collect::<String>();
    let suffix = chars[target_end + 1..close].iter().collect::<String>();
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

fn previous_char_boundary(text: &str, byte_index: usize) -> usize {
    text[..byte_index]
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn remove_suffix_ruby(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;
    while index < chars.len() {
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

    let target_chars = suffix_visible_text(target).chars().collect::<Vec<_>>();
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
    Some(render_gaiji_replacement(&replacement, config))
}

fn render_gaiji_replacement(input: &str, config: &AozoraConfig) -> String {
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
        index += push_text_char(&mut output, &chars, index, config, false);
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
        escape_html(&base.to_string())
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
        return Some((end, escape_html(&note)));
    }
    let normalized_note = crate::config::normalize_gaiji_key(&note);
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
        return Some((end, render_gaiji_replacement(replacement, config)));
    }
    if let Some(replacement) = unicode_replacement(bare_note, config) {
        return Some((end, replacement));
    }
    Some((end, escape_html(&note)))
}

fn gaiji_note_range(chars: &[char], start: usize) -> Option<(usize, String)> {
    if chars.get(start) != Some(&'※')
        || chars.get(start + 1) != Some(&'［')
        || chars.get(start + 2) != Some(&'＃')
    {
        return None;
    }
    let mut depth = 1usize;
    let mut index = start + 3;
    while index < chars.len() {
        if chars.get(index) == Some(&'［') && chars.get(index + 1) == Some(&'＃') {
            depth += 1;
            index += 2;
            continue;
        }
        if chars[index] == '］' {
            depth -= 1;
            if depth == 0 {
                let note = chars[start..=index].iter().collect::<String>();
                return Some((index + 1, note));
            }
        }
        index += 1;
    }
    None
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
            .unwrap_or_else(|| format!("<img class=\"gaiji\" src=\"{source}\" alt=\"\"/>"));
        return Some((end, replacement));
    }
    let alt = if config.inline_notes.contains_key("画像") && !description.is_empty() {
        escape_html(&description)
    } else {
        "挿絵".to_owned()
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

fn is_external_reference(value: &str) -> bool {
    let value = value.trim();
    if value.starts_with("//") {
        return true;
    }
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && scheme.chars().enumerate().all(|(index, character)| {
            if index == 0 {
                character.is_ascii_alphabetic()
            } else {
                character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
            }
        })
}

fn parse_raw_anchor(chars: &[char], start: usize) -> Option<(usize, String)> {
    let end = chars
        .iter()
        .enumerate()
        .skip(start + 1)
        .find_map(|(index, character)| (*character == '>').then_some(index))?;
    let raw = chars[start..=end].iter().collect::<String>();
    let lower = raw.to_ascii_lowercase();
    let replacement = if lower == "<a>" || lower == "</a>" {
        raw
    } else if lower.starts_with("<a") {
        if let Some(name) = raw_tag_attribute(&raw, "name") {
            let name = name.trim();
            if name.is_empty() || name.contains('<') || name.contains('>') {
                return None;
            }
            format!("<a id=\"{}\">", escape_html(name))
        } else if let Some(href) = raw_tag_attribute(&raw, "href") {
            if href.contains('"') || href.contains('<') || href.contains('>') {
                return None;
            }
            if is_external_reference(href) {
                "<a>".to_owned()
            } else {
                format!("<a href=\"{}\">", escape_html(href))
            }
        } else {
            return None;
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
        && !(note.contains("」に「") && !note.contains("ママ"))
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
            push_escaped_char(output, composed);
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
            push_escaped_char(output, chars[index]);
            output.push_str("<span>");
            push_escaped_char(output, mark);
            output.push_str("</span></span>");
            return 2;
        }
    }

    let character = normalize_vertical_character(chars[index], config.vertical);
    if allow_upright && is_upright_character(character) {
        output.push_str("<span class=\"upr\">");
        push_escaped_char(output, character);
        output.push_str("</span>");
    } else {
        push_escaped_char(output, character);
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
        0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x2ffff => Some(0),
        0x3041..=0x3096 => Some(1),
        0x30a1..=0x30fa | 0xff61..=0xff9f => Some(2),
        0x20..=0x7e | 0xa0..=0x2af => Some(3),
        0xff10..=0xff19 | 0xff21..=0xff3a | 0xff41..=0xff5a => Some(4),
        _ => None,
    }
}

fn push_ruby(
    output: &mut String,
    base: &str,
    reading: &str,
    config: &AozoraConfig,
    allow_auto_yoko: bool,
) {
    output.push_str("<ruby>");
    if allow_auto_yoko && !contains_literal_gaiji_note(base) {
        output.push_str(&convert_inline(base, config));
    } else {
        output.push_str(&convert_inline_without_auto_yoko(base, config));
    }
    output.push_str("<rt>");
    output.push_str(&convert_ruby_reading(reading, config));
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
        _ => output.push(character),
    }
}
