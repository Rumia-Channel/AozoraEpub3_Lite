use super::{
    aozora_text_to_xhtml_sections, aozora_text_to_xhtml_sections_with_config, decode_input,
    image_reference_occurrences, image_references, plain_text_to_xhtml,
};
use crate::config::{AozoraConfig, IniSettings};
use encoding_rs::SHIFT_JIS;
use std::path::PathBuf;

#[test]
fn escapes_text_and_preserves_empty_lines() {
    let output = plain_text_to_xhtml("A<&>'\n\nB").unwrap();
    assert_eq!(
        output,
        "    <p>A&lt;&amp;&gt;'</p>\n    <p><br/></p>\n    <p>B</p>\n"
    );
}

#[test]
fn preserves_or_converts_comment_blocks_per_config() {
    let input = "前\n--------------------------------------------------\n<raw>&［＃太字］注記［＃太字終わり］\n--------------------------------------------------\n後";
    let hidden = plain_text_to_xhtml(input).unwrap();
    assert!(!hidden.contains("raw"));

    let raw_config =
        AozoraConfig::from_ini(IniSettings::parse("CommentPrint=1\nCommentConvert=0\n").unwrap());
    let raw = super::plain_text_to_xhtml_with_config(input, &raw_config).unwrap();
    assert!(raw.contains("&lt;raw&gt;&amp;［＃太字］注記［＃太字終わり］"));
    assert!(!raw.contains("<span class=\"bold\">"));

    let converted_config =
        AozoraConfig::from_ini(IniSettings::parse("CommentPrint=1\nCommentConvert=1\n").unwrap());
    let converted = super::plain_text_to_xhtml_with_config(input, &converted_config).unwrap();
    assert!(converted.contains("<span class=\"bold\">注記</span>"));
}

#[test]
fn normalizes_java_vertical_text_characters() {
    let output = plain_text_to_xhtml("≪≫“”〝〟―").unwrap();
    assert!(output.contains("《》〝〟〝〟─"));
    assert!(!output.contains("≪"));
    assert!(!output.contains("≫"));
}

#[test]
fn applies_empty_line_limits_and_force_page_breaks() {
    let config = AozoraConfig::from_ini(
        IniSettings::parse("RemoveEmptyLine=1\nMaxEmptyLine=2\nPageBreak=1\nPageBreakSize=1\n")
            .unwrap(),
    );
    let output = super::plain_text_to_xhtml_with_config("前\n\n\n\n後", &config).unwrap();
    assert_eq!(output.matches("<p><br/></p>").count(), 2);

    let long_line = "あ".repeat(600);
    let sections =
        aozora_text_to_xhtml_sections_with_config(&format!("{long_line}\n後"), &config).unwrap();
    assert_eq!(sections.len(), 2);
    assert!(sections[1].contains("<p>後</p>"));
}

#[test]
fn applies_java_force_indent_rules() {
    let config = AozoraConfig::from_ini(IniSettings::parse("ForceIndent=1").unwrap());
    let output =
        super::plain_text_to_xhtml_with_config("本文\n「会話\n 本文\n  本文", &config).unwrap();
    assert!(output.contains("<p>　本文</p>"));
    assert!(output.contains("<p>「会話</p>"));
    assert!(output.contains("<p>　本文</p>"));
    assert!(output.contains("<p> 　本文</p>"));
}

#[test]
fn adds_kobo_bookmark_ids_to_paragraphs() {
    let config = AozoraConfig::from_ini(IniSettings::parse("MarkId=1").unwrap());
    let output = super::plain_text_to_xhtml_with_config("本文\n\n次", &config).unwrap();
    assert!(output.contains("<p id=\"kobo.1.1\">本文</p>"));
    assert!(output.contains("<p id=\"kobo.2.1\"><br/></p>"));
    assert!(output.contains("<p id=\"kobo.3.1\">次</p>"));
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
fn groups_java_kanji_in_implicit_ruby_bases() {
    let output = plain_text_to_xhtml("〆切《しめきり》 漢字𥈽漢字《かんじがいじかんじ》").unwrap();
    assert!(output.contains("<ruby>〆切<rt>しめきり</rt></ruby>"));
    assert!(output.contains("<ruby>漢字𥈽漢字<rt>かんじがいじかんじ</rt></ruby>"));
}
#[test]
fn groups_non_kanji_implicit_ruby_bases() {
    let output =
        plain_text_to_xhtml("漢字AB《えーびー》 漢字ひらがな《ひらがな》 カタカナ《かたかな》")
            .unwrap();
    assert!(output.contains("漢字<ruby>AB<rt>えーびー</rt></ruby>"));
    assert!(output.contains("漢字<ruby>ひらがな<rt>ひらがな</rt></ruby>"));
    assert!(output.contains("<ruby>カタカナ<rt>かたかな</rt></ruby>"));
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
fn splits_page_breaks_embedded_in_text_lines() {
    let sections = aozora_text_to_xhtml_sections("前［＃改ページ］後［＃改ページ］終").unwrap();
    assert_eq!(sections.len(), 3);
    assert!(sections[0].contains("<p>前</p>"));
    assert!(sections[1].contains("<p>後</p>"));
    assert!(sections[2].contains("<p>終</p>"));
}
#[test]
fn separates_colophon_into_a_non_chapter_section() {
    let sections = aozora_text_to_xhtml_sections("本文\n底本：青空文庫").unwrap();
    assert_eq!(sections.len(), 2);
    assert!(sections[0].contains("<p>本文</p>"));
    assert!(sections[1].starts_with("<!-- aozora-page-no-chapter -->"));
    assert!(sections[1].contains("<p>底本：青空文庫</p>"));
}

#[test]
fn classifies_middle_and_bottom_page_breaks() {
    let mut config = AozoraConfig::default();
    config.load_tag_text("中寄せ\t\tM\n下寄せ\t\tL\n");
    let sections = super::aozora_text_to_xhtml_sections_with_config(
        "前\n［＃中寄せ］\n中央\n［＃下寄せ］\n下",
        &config,
    )
    .unwrap();
    assert!(sections[1].starts_with("<!-- aozora-page-middle -->"));
    assert!(sections[2].starts_with("<!-- aozora-page-bottom -->"));
}

#[test]
fn converts_and_collects_safe_raw_image_tags() {
    let input = r#"<img src="fig/sample.png" alt="図"/>"#;
    let output = plain_text_to_xhtml(input).unwrap();
    assert!(output.contains("<img class=\"fit\" src=\"../image/fig/sample.png\" alt=\"図\"/>"));
    assert_eq!(image_references(input), vec!["fig/sample.png"]);
}

#[test]
fn preserves_image_reference_order_and_occurrences() {
    let input =
        "［＃図（img/note.png）入る］\n<Img Src=\"img/raw.png\">\n［＃図（img/note.png）入る］";
    assert_eq!(
        image_reference_occurrences(input),
        vec!["img/note.png", "img/raw.png", "img/note.png"]
    );
    assert_eq!(image_references(input), vec!["img/note.png", "img/raw.png"]);
}

#[test]
fn hides_and_optionally_keeps_comment_blocks() {
    let input = "本文\n--------------------------------------------------\n内部\n--------------------------------------------------\n後";
    let hidden = plain_text_to_xhtml(input).unwrap();
    assert!(hidden.contains("<p>本文</p>"));
    assert!(hidden.contains("<p>後</p>"));
    assert!(!hidden.contains("内部"));

    let config = AozoraConfig {
        comment_print: true,
        ..AozoraConfig::default()
    };
    let printed = super::plain_text_to_xhtml_with_config(input, &config).unwrap();
    assert!(printed.contains("内部"));
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
fn auto_vertical_layout_markup_is_not_escaped() {
    let output = plain_text_to_xhtml("あ12い!?う").unwrap();
    assert!(output.contains("<span class=\"tcy\">12</span>"));
    assert!(output.contains("<span class=\"tcy\">!?</span>"));
    assert!(!output.contains("&lt;span class=&quot;tcy&quot;"));
}

#[test]
fn auto_combines_ruby_bases_when_auto_yoko_is_enabled() {
    let output = plain_text_to_xhtml("｜29《二十九》").unwrap();
    assert!(output.contains("<ruby><span class=\"tcy\">29</span><rt>二十九</rt></ruby>"));

    let explicit = plain_text_to_xhtml("｜29《二十九》［＃「29」は縦中横］").unwrap();
    assert!(explicit.contains("<span class=\"tcy\"><ruby>29<rt>二十九</rt></ruby></span>"));
    assert!(!explicit.contains("<ruby><span class=\"tcy\">29</span>"));
}
#[test]
fn keeps_auto_yoko_markup_inside_horizontal_blocks() {
    let output =
        plain_text_to_xhtml("［＃ここから横組み］\n横組み100円\n［＃ここで横組み終わり］").unwrap();
    assert!(output.contains("<span class=\"tcy\">100</span>"));
    assert!(!output.contains("&lt;span class=&quot;tcy&quot;"));
}

#[test]
fn keeps_suffix_tcy_notes_outside_following_ruby() {
    let output = plain_text_to_xhtml("Ｂ｜29［＃「29」は縦中横］《二十九》").unwrap();
    assert!(output.contains("Ｂ<span class=\"tcy\"><ruby>29<rt>二十九</rt></ruby></span>"));
    assert!(!output.contains("<ruby>29</ruby></span><rt>"));
}

#[test]
fn converts_gaiji_notes_inside_ruby_readings() {
    let input = "｜漢字《※［＃米印］》";
    let output = plain_text_to_xhtml(input).unwrap();
    assert!(output.contains("<ruby>漢字<rt>※</rt></ruby>"));
}

#[test]
fn converts_gaiji_brackets_inside_ruby() {
    let output = plain_text_to_xhtml(
        "｜※［＃始め二重山括弧］29※［＃終わり二重山括弧］《※［＃始め二重山括弧］29※［＃終わり二重山括弧］》",
    )
    .unwrap();
    assert!(output.contains("<ruby>《29》<rt>《29》</rt></ruby>"));
    assert!(!output.contains("<ruby>《29<rt>"));
}

#[test]
fn converts_inline_notes_inside_ruby_bases() {
    let output =
        plain_text_to_xhtml("｜あいう［＃縦中横］1［＃縦中横終わり］《ふりがな》").unwrap();
    assert!(output.contains("<ruby>あいう<span class=\"tcy\">1</span><rt>ふりがな</rt></ruby>"));
    assert!(!output.contains("［＃縦中横］"));
}
#[test]
fn vertical_symbols_and_combining_dakuten_match_default_output() {
    let output = plain_text_to_xhtml("☆÷あ゛は゜").unwrap();
    assert!(output.contains("<span class=\"upr\">☆</span>"));
    assert!(output.contains("<span class=\"upr\">÷</span>"));
    assert!(output.contains("あ<span>゛</span>"));
    assert!(output.contains("ぱ"));
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
    assert!(output.contains("<img class=\"fit\" src=\"../image/fig/sample.png\" alt=\"sample\"/>"));
    assert_eq!(image_references(input), vec!["fig/sample.png"]);
}

#[test]
fn converts_gaiji_image_notes_without_remote_resources() {
    let input = "※［＃外字（gaiji.png）#GAIJI#］";
    let output = plain_text_to_xhtml(input).unwrap();
    assert!(output.contains("<img class=\"gaiji\" src=\"../image/gaiji.png\" alt=\"\"/>"));
    assert_eq!(image_references(input), vec!["gaiji.png"]);
}

#[test]
fn converts_raw_named_anchors_to_xhtml_ids() {
    let output = plain_text_to_xhtml(r##"<a name="aaa">本文</a><a href="#aaa">参照</a>"##).unwrap();
    assert!(output.contains(r#"<a id="aaa">本文</a>"#));
    assert!(output.contains(r##"<a href="#aaa">参照</a>"##));
}

#[test]
fn strips_external_raw_anchor_targets() {
    let output = plain_text_to_xhtml(
        r#"<a href="https://example.com/book">外部リンク</a><a href="//cdn.example.com">CDN</a>"#,
    )
    .unwrap();
    assert!(output.contains("<a>外部リンク</a>"));
    assert!(output.contains("<a>CDN</a>"));
    assert!(!output.contains("https://"));
    assert!(!output.contains("//cdn.example.com"));
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
    assert!(block.contains("<h2 class=\"font-1em30\">\n<p>章題</p>\n</h2>"));
    assert!(block.contains("<p>本文</p>"));
}
#[test]
fn renders_basic_indent_blocks() {
    let output =
        plain_text_to_xhtml("［＃ここから１字下げ］\n字下げ本文\n［＃ここで字下げ終わり］")
            .unwrap();
    assert!(output.contains("<div class=\"mt1\">\n<p>字下げ本文</p>\n</div>"));
}

#[test]
fn closes_configured_heading_inside_indent_block() {
    let mut config = AozoraConfig::default();
    config.load_tag_text(
        "３字下げ\t<div class=\"mt3\">\t</div>\t1\n\
             中見出し\t<h2 class=\"font-1em30\">\t</h2>\t1\n",
    );
    let output = super::plain_text_to_xhtml_with_config(
        "［＃３字下げ］［＃中見出し］章題［＃中見出し終わり］",
        &config,
    )
    .unwrap();
    assert!(output.contains("<div class=\"mt3\"><h2 class=\"font-1em30\">章題</h2></div>"));
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
    assert!(output.contains("<div class=\"bold\">\n<p>本文</p>\n</div>"));
    assert!(output.contains("<h1 class=\"custom\">題名</h1>"));
    assert!(output.contains("<p><br/></p>"));
}

#[test]
fn separates_inline_block_markers_from_paragraphs() {
    let mut config = AozoraConfig::default();
    config.load_tag_text(
        "ここから後書き\t<div class=\"postscript\">\t\t1\n\
             ここで後書き終わり\t</div><div class=\"clear\"></div>\t\t1\n",
    );
    let output = super::plain_text_to_xhtml_with_config(
        "本文［＃ここから後書き］\n次［＃小書き］回［＃小書き終わり］\n\
             ［＃ここで後書き終わり］",
        &config,
    )
    .unwrap();
    assert!(output.contains("<p>本文</p>\n<div class=\"postscript\">"));
    assert!(output.contains("次<span class=\"kogaki\">回</span>"));
    assert!(!output.contains("［＃"));
}

#[test]
fn converts_default_grounding_and_special_brackets() {
    let output = plain_text_to_xhtml(
        "［＃ここから地付き］［＃小書き］注記［＃小書き終わり］\
             ［＃ここで地付き終わり］ ※［＃始め二重山括弧］本文※［＃終わり二重山括弧］",
    )
    .unwrap();
    assert!(
        output.contains("<div class=\"btm\">\n<p><span class=\"kogaki\">注記</span></p>\n</div>")
    );
    assert!(output.contains("《本文》"));
    assert!(!output.contains("［＃"));
}

#[test]
fn inserts_warichu_line_breaks_like_java() {
    let output = plain_text_to_xhtml(
        "［＃割り注］ヒロソヒイ［＃割り注終わり］\n\
         ［＃割り注］東は字大林四三七［＃改行］西は字神内一一一ノ一［＃割り注終わり］\n\
         ［＃割り注］〔二十三日〕［＃割り注終わり］\n\
         ［＃割り注］ポリチカルマタル。［＃割り注終わり］",
    )
    .unwrap();
    assert!(output.contains("<span class=\"wrc\">ヒロソ<br/>ヒイ</span>"));
    assert!(
        output.contains("<span class=\"wrc\">東は字大林四三七<br/>西は字神内一一一ノ一</span>")
    );
    assert!(output.contains("〔<span class=\"wrc\">二十<br/>三日</span>〕"));
    assert!(output.contains("<span class=\"wrc\">ポリチカル<br/>マタル。</span>"));
    assert!(!output.contains("［＃"));
}

#[test]
fn renders_java_compound_indent_blocks() {
    let output = plain_text_to_xhtml(
        "［＃ここから５字下げ、折り返して２字下げ］\n\
         折り返し本文\n\
         ［＃ここで字下げ終わり］\n\
         ［＃ここから３字下げ、４字詰め］\n\
         字詰め本文\n\
         ［＃ここで字下げ終わり］\n\
         ［＃ここから２字下げ、罫囲みと中央揃え］\n\
         複合本文\n\
         ［＃ここで字下げ終わり］",
    )
    .unwrap();
    assert!(output.contains("<div class=\"pt2 idt3\">\n<p>折り返し本文</p>\n</div>"));
    assert!(output.contains("<div class=\"pt3 jzm4\">\n<p>字詰め本文</p>\n</div>"));
    assert!(output.contains("<div class=\"mt2 border center\">\n<p>複合本文</p>\n</div>"));
    assert!(!output.contains("［＃"));
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
    assert!(output.contains(
        "<div class=\"mt2\">\n<div class=\"bold\">\n<p>本文</p>\n<p><br/></p>\n</div>\n</div>"
    ));
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
    assert!(
        output.contains(
            "<span class=\"em-line\"><span class=\"bold\"><span class=\"em-sesame\">青空"
        )
    );
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
fn resolves_unicode_code_after_unknown_gaiji_note() {
    let output =
        plain_text_to_xhtml("※［＃「」、第4水準2-1-1］「」、第4水準2-1-1 U+20089").unwrap();
    assert!(output.contains("𠂉"));
    assert!(!output.contains("行右小書き"));
}

#[test]
fn emits_configured_dakuten_glyph_font() {
    let mut config = AozoraConfig {
        dakuten_type: 2,
        ..Default::default()
    };
    config.gaiji_fonts.insert(
        "u3048-u3099".to_owned(),
        PathBuf::from("dakuten/u3048-u3099.ttf"),
    );
    let output =
        super::plain_text_to_xhtml_with_config("※［＃濁点付き平仮名え］", &config).unwrap();
    assert!(output.contains("<span class=\"glyph u3048-u3099\">え</span>"));
    let located =
        super::plain_text_to_xhtml_with_config("※［＃濁点付き平仮名え、ページ数-行数］", &config)
            .unwrap();
    assert!(located.contains("<span class=\"glyph u3048-u3099\">え</span>"));
}
#[test]
fn emits_configured_dakuten_glyph_fonts_for_dash_and_greek_iota() {
    let mut config = AozoraConfig {
        dakuten_type: 2,
        ..Default::default()
    };
    config
        .gaiji_fonts
        .insert("u30fc-u3099".to_owned(), PathBuf::from("u30fc-u3099.ttf"));
    config
        .gaiji_fonts
        .insert("u3b9-u3099".to_owned(), PathBuf::from("u3b9-u3099.ttf"));
    let output = super::plain_text_to_xhtml_with_config("ー゛ι゛", &config).unwrap();
    assert!(output.contains("<span class=\"glyph u30fc-u3099\">ー</span>"));
    assert!(output.contains("<span class=\"glyph u3b9-u3099\">ι</span>"));
}

#[test]
fn emits_configured_unicode_glyph_font() {
    let mut config = AozoraConfig::default();
    config
        .gaiji_fonts
        .insert("u845b".to_owned(), PathBuf::from("u845b.ttf"));
    let output = super::plain_text_to_xhtml_with_config("※［＃U+845B］", &config).unwrap();
    assert!(output.contains("<span class=\"glyph u845b\">〓</span>"));
}
#[test]
fn normalizes_shift_jis_dash_variants_in_gaiji_notes() {
    let mut config = AozoraConfig::default();
    config.load_ivs_text("U+0001\tU+E0101\t検\t※［＃「foo－bar」］\n");
    let output = super::plain_text_to_xhtml_with_config("※［＃「foo―bar」］", &config).unwrap();
    assert!(output.contains("<p>検</p>"));
    assert!(!output.contains("行右小書き"));
}

#[test]
fn resolves_gaiji_aliases_with_spacing_and_ascii_separators() {
    let mut config = AozoraConfig::default();
    config.load_utf_text("U+0001\t\t検\t※［＃fooBar,説明］\n");
    let output = super::plain_text_to_xhtml_with_config("※［＃foo Bar、説明］", &config).unwrap();
    assert!(output.contains("<p>検</p>"));
    assert!(!output.contains("行右小書き"));
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
    let output =
        super::plain_text_to_xhtml_with_config("※［＃感嘆符三つ］ ※［＃小書き平仮名こ］", &config)
            .unwrap();
    assert!(output.contains("<span class=\"tcy\">!!!</span>"));
    assert!(output.contains("<span class=\"kogaki\">こ</span>"));
    assert!(!output.contains("※［＃"));
}

#[test]
fn converts_external_latin_decomposition_inside_brackets() {
    let mut config = AozoraConfig::default();
    config.load_latin_text("A`\tÀ\nAE&\tÆ\n");
    let output = super::plain_text_to_xhtml_with_config("〔A` AE&〕 〔漢字〕", &config).unwrap();
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
    assert!(output.contains("<span class=\"em-sesame\"><ruby>青空<rt>あおぞら</rt></ruby></span>"));
}

#[test]
fn converts_suffix_ruby_notes() {
    let output = plain_text_to_xhtml(
        "青空文庫［＃「文庫」に「ぶんこ」のルビ］\n\
         青空文庫［＃「青空文庫」に「aozora bunko」のルビ］\n\
         漢字青空文庫《あおぞらぶんこ》［＃「青空文庫《あおぞらぶんこ》」に「aozora bunko」のルビ］",
    )
    .unwrap();
    assert!(output.contains("青空<ruby>文庫<rt>ぶんこ</rt></ruby>"));
    assert!(output.contains("<ruby>漢字青空文庫<rt>aozora bunko</rt></ruby>"));
    assert!(output.contains("<ruby>青空文庫<rt>aozora bunko</rt></ruby>"));
    assert!(!output.contains("のルビ"));
}

#[test]
fn converts_note_attached_ruby_markers() {
    let output = plain_text_to_xhtml(
        "［＃注記付き］名※［＃二の字点、1-2-22］［＃「（銘々）」の注記付き終わり］",
    )
    .unwrap();
    assert!(output.contains("<ruby>"));
    assert!(output.contains("<rt>（銘々）</rt>"));
    assert!(!output.contains("注記付き"));
}

#[test]
fn drops_java_unsupported_left_ruby_notes() {
    let output = plain_text_to_xhtml(
        "青空文庫［＃「青空文庫」の左に「あおぞらぶんこ」のルビ］\n\
         ［＃左にルビ付き］欞子窓［＃左に「れんじまど」のルビ付き終わり］\n\
         ［＃左に注記付き］名※［＃二の字点、1-2-22］［＃左に「（銘々）」の注記付き終わり］",
    )
    .unwrap();
    assert!(output.contains("青空文庫"));
    assert!(output.contains("欞子窓"));
    assert!(!output.contains("左に"));
    assert!(!output.contains("のルビ"));
}

#[test]
fn preserves_java_unconverted_notes_and_drops_unknown_markers() {
    let output = plain_text_to_xhtml(
        "大空文庫［＃「大空文庫」に「ママ」の注記］\n\
         本文［＃注記未定義］\n\
         責［＃「責」の左に未知傍点］空文庫",
    )
    .unwrap();
    assert!(output.contains("大空文庫"));
    assert!(output.contains("<p>本文</p>"));
    assert!(output.contains("<p>責空文庫</p>"));
    assert!(!output.contains("注記未定義"));
    assert!(!output.contains("未知傍点"));
}
