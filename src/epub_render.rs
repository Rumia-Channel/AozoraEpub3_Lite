use super::{EpubAsset, EpubMetadata, EpubSection, NavChapter, TITLE_PAGE_MARKER, is_title_page};
const PAGE_MIDDLE_MARKER: &str = "<!-- aozora-page-middle -->";
const PAGE_BOTTOM_MARKER: &str = "<!-- aozora-page-bottom -->";
const PAGE_NO_CHAPTER_MARKER: &str = "<!-- aozora-page-no-chapter -->";
const PAGE_CHAPTER_MARKER: &str = "<!-- aozora-page-chapter -->";

/// One TOC row with the nesting fields Java's `ChapterInfo.setTocNestLevel`
/// computes: `level` is the raw heading level going in and the nesting depth
/// coming out; `level_start`/`level_end` count `<ol>` opens and
/// `</li></ol>` closes for nav.xhtml; `nav_close` counts `</navPoint>`
/// closes for toc.ncx.
#[derive(Clone, Debug)]
struct TocEntry {
    label: String,
    markup: bool,
    path: String,
    level: usize,
    level_start: usize,
    level_end: usize,
    nav_close: usize,
}

/// Port of `ChapterInfo.setTocNestLevel`: converts raw heading levels into
/// nesting depths by counting preceding entries with a strictly smaller
/// level, then derives the Velocity counters. `title_toc` mirrors
/// `insertTitleToc`: the first entry (the title) adopts the second entry's
/// level so it nests as a sibling instead of a root.
fn set_toc_nest_level(entries: &mut [TocEntry], ncx_nest: bool, title_toc: bool) {
    if entries.is_empty() {
        return;
    }
    if title_toc && entries.len() >= 2 {
        entries[0].level = entries[1].level;
    }
    let levels: Vec<usize> = entries.iter().map(|entry| entry.level).collect();
    for (index, entry) in entries.iter_mut().enumerate() {
        let mut count = 0usize;
        let mut current = levels[index];
        for &level in levels[..index].iter().rev() {
            if level < current {
                count += 1;
                current = level;
            }
        }
        entry.level = count;
    }
    entries[0].level_start = 0;
    for index in 1..entries.len() {
        let (before, after) = entries.split_at_mut(index);
        let prev = &mut before[index - 1];
        let curr = &mut after[0];
        if curr.level > prev.level {
            curr.level_start = curr.level - prev.level;
            prev.level_end = 0;
        } else {
            curr.level_start = 0;
            prev.level_end = prev.level - curr.level;
        }
    }
    let last = entries.len() - 1;
    entries[last].level_end = entries[last].level;
    if ncx_nest {
        for index in 0..entries.len() {
            entries[index].nav_close = entries[index].level_end + 1;
            if entries[index].level_start > 0 && index > 0 {
                entries[index - 1].nav_close = 0;
            }
        }
    }
}

fn asset_manifest_id(path: &str, fallback: usize) -> String {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let stem = filename.split('.').next().unwrap_or(filename);
    stem.parse::<u32>()
        .map(|number| format!("img{number:04}"))
        .unwrap_or_else(|_| format!("img{fallback:04}"))
}

pub(super) fn render_package(
    metadata: &EpubMetadata,
    sections: &[EpubSection],
    assets: &[EpubAsset],
    cover_asset: Option<&str>,
    vertical: bool,
    toc_page: bool,
    insert_cover_page: bool,
) -> String {
    let identifier = metadata
        .identifier
        .strip_prefix("urn:uuid:")
        .or_else(|| metadata.identifier.strip_prefix("urn:"))
        .unwrap_or(&metadata.identifier);
    let image_only = is_image_only(sections);
    let creator = metadata
        .creator
        .as_deref()
        .map(|value| {
            format!(
                "\n\n<!-- 著者名 -->\n\t\t<dc:creator id=\"creator01\">{}</dc:creator>",
                xml_escape(value)
            )
        })
        .unwrap_or_default();
    let publisher = metadata
        .publisher
        .as_deref()
        .map(|value| {
            format!(
                "\n<!-- 出版社名 -->\n\t\t<dc:publisher id=\"publisher\">{}</dc:publisher>",
                xml_escape(value)
            )
        })
        .unwrap_or_default();
    let fixed_metadata = if image_only {
        format!(
            "\n\n\t\t<!-- Fixed-Layout Documents指定 -->\n\
\t\t<meta property=\"rendition:layout\">pre-paginated</meta>\n\
\t\t<meta property=\"rendition:spread\">landscape</meta>\n\
\t\t<meta name=\"original-resolution\" content=\"${{coverImage.Width}}x${{coverImage.Height}}\"/>\n\
\n\
\t\t<meta name=\"primary-writing-mode\" content=\"{}\"/>",
            if vertical {
                "horizontal-rl"
            } else {
                "horizontal-lr"
            }
        )
    } else {
        String::new()
    };
    let styles = if image_only {
        "\t\t<item id=\"svg_image\" href=\"style/fixed-layout-jp.css\" media-type=\"text/css\"/>\n"
    } else {
        "\t\t<item id=\"vertical\" href=\"style/aozora.css\" media-type=\"text/css\"/>\n\
        \t\t<item id=\"v_font\" href=\"style/font.css\" media-type=\"text/css\"/>\n\
        \t\t<item id=\"v_text\" href=\"style/text.css\" media-type=\"text/css\"/>\n\
        \t\t<item id=\"fixed-layout-jp\" href=\"style/fixed-layout-jp.css\" media-type=\"text/css\"/>\n\
        \t\t<item id=\"book-style\" href=\"style/book-style.css\" media-type=\"text/css\"/>\n\
        \t\t<item id=\"style-reset\" href=\"style/style-reset.css\" media-type=\"text/css\"/>\n\
        \t\t<item id=\"style-standard\" href=\"style/style-standard.css\" media-type=\"text/css\"/>\n\
        \t\t<item id=\"style-advance\" href=\"style/style-advance.css\" media-type=\"text/css\"/>\n"
    };
    let mut body_number = 0;
    let mut manifest_sections = String::new();
    let mut spine_sections = String::new();
    // Java package.vm: xhtml セクション群の直前には常に空行が1行入る
    // （#if/#end の直後のリテラル空行）。表題ページと nav の item はその前に出る。
    let mut manifest_head = String::new();
    let mut spine_head = String::new();
    let mut nav_spine_inserted = false;
    for (index, section) in sections.iter().enumerate() {
        if is_title_page(section) {
            manifest_head.push_str(
                "\t\t<item id=\"title-page\" href=\"xhtml/title.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
            );
            spine_head.push_str("\t\t<itemref idref=\"title-page\" linear=\"yes\"/>\n");
            // Java package.vm: InsertTocPage places the nav itemref right
            // after the title page, before the body sections.
            if toc_page {
                spine_head.push_str("\t\t<itemref idref=\"nav\" linear=\"yes\"/>\n");
                nav_spine_inserted = true;
            }
            continue;
        }
        body_number += 1;
        if image_only {
            manifest_sections.push_str(&format!(
                "\t\t<item media-type=\"application/xhtml+xml\" id=\"sec{body_number:04}\" href=\"xhtml/{body_number:04}.xhtml\" properties=\"svg\"/>\n"
            ));
        } else {
            manifest_sections.push_str(&format!(
                "\t\t<item id=\"sec{body_number:04}\" href=\"xhtml/{body_number:04}.xhtml\" media-type=\"application/xhtml+xml\"/>\n"
            ));
        }
        let spread = if image_only {
            let right = if vertical {
                index % 2 == 0
            } else {
                index % 2 != 0
            };
            if right {
                " properties=\"page-spread-right\""
            } else {
                " properties=\"page-spread-left\""
            }
        } else {
            ""
        };
        spine_sections.push_str(&format!(
            "\t\t<itemref linear=\"yes\" idref=\"sec{body_number:04}\"{spread}/>\n"
        ));
    }
    if toc_page && !nav_spine_inserted {
        // No title page: the nav itemref still precedes the body sections.
        spine_head.push_str("\t\t<itemref idref=\"nav\" linear=\"yes\"/>\n");
    }
    let mut manifest_assets = String::new();
    // Java: 外字フォントの item は xhtml セクション群の後、ncx の前に出力される
    let mut manifest_gaiji = String::new();
    let mut gaiji_number = 0;
    for (index, asset) in assets.iter().enumerate() {
        // Java: `properties="cover-image"` は表紙ページを出力するときだけ付く
        // (Epub3Writer の insertCoverPage ブロックで setIsCover(true) される)。
        let properties =
            if !image_only && insert_cover_page && cover_asset == Some(asset.path.as_str()) {
                " properties=\"cover-image\""
            } else {
                ""
            };
        if asset.path.starts_with("gaiji/") {
            gaiji_number += 1;
            manifest_gaiji.push_str(&format!(
                "\t\t<item id=\"gaiji_{gaiji_number}\" href=\"{}\" media-type=\"{}\"{properties}/>\n",
                xml_escape(&asset.path),
                xml_escape(&asset.media_type),
            ));
            continue;
        }
        let id = asset_manifest_id(&asset.path, index + 1);
        // Java package.vm は表紙画像だけ属性の順序が異なる。
        if properties.is_empty() {
            manifest_assets.push_str(&format!(
                "\t\t<item id=\"{id}\" href=\"{}\" media-type=\"{}\"{properties}/>\n",
                xml_escape(&asset.path),
                xml_escape(&asset.media_type),
            ));
        } else {
            manifest_assets.push_str(&format!(
                "\t\t<item media-type=\"{}\" id=\"{id}\" href=\"{}\"{properties}/>\n",
                xml_escape(&asset.media_type),
                xml_escape(&asset.path),
            ));
        }
    }
    let cover_manifest = if !image_only && insert_cover_page {
        // Java package.vm: 表紙 item の直後にリテラル空行が1行入る
        "\t\t<item media-type=\"application/xhtml+xml\" id=\"cover-page\" href=\"xhtml/cover.xhtml\" properties=\"svg\"/>\n\n"
    } else {
        ""
    };
    let cover_spine = if !image_only && insert_cover_page {
        "\t\t   <itemref linear=\"yes\" idref=\"cover-page\" properties=\"rendition:page-spread-center\"/>\n"
    } else {
        ""
    };
    let progression = if vertical { "rtl" } else { "ltr" };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package
 xmlns="http://www.idpf.org/2007/opf"
 version="3.0"
 xml:lang="{language}"
 unique-identifier="unique-id"
 prefix="rendition: http://www.idpf.org/vocab/rendition/#
         ebpaj: http://www.ebpaj.jp/
         fixed-layout-jp: http://www.digital-comic.jp/
         ibooks: http://vocabulary.itunes.apple.com/rdf/ibooks/vocabulary-extensions-1.0/"
>
		<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<!-- 作品名 -->
		<dc:title id="title">{title}</dc:title>{creator}{publisher}
<!-- 言語 -->
		<dc:language id="pub-lang">{language}</dc:language>
<!-- ファイルid -->
		<dc:identifier id="unique-id">urn:uuid:{identifier}</dc:identifier>
<!-- 更新日 -->
		<meta property="dcterms:modified">{modified}</meta>{fixed_metadata}

<!-- etc. -->
<meta property="ebpaj:guide-version">1.1.3</meta>
<meta property="ibooks:version">1.1.2</meta>
	</metadata>

	<manifest>
<!-- navigation -->
		<item media-type="application/xhtml+xml" id="nav" href="nav.xhtml" properties="nav"/>
<!-- style -->
{styles}<!-- image -->
{assets}<!-- xhtml -->
{cover}{head}
{sections}{gaiji}		<item href="toc.ncx" id="ncx" media-type="application/x-dtbncx+xml"/>
	</manifest>

	<spine page-progression-direction="{progression}" toc="ncx">
{cover_spine}{spine_head}
{spine}	</spine>

</package>"#,
        language = xml_escape(&metadata.language),
        title = xml_escape(&metadata.title),
        creator = creator,
        publisher = publisher,
        identifier = xml_escape(identifier),
        modified = xml_escape(&metadata.modified),
        fixed_metadata = fixed_metadata,
        styles = styles,
        assets = manifest_assets,
        cover = cover_manifest,
        head = manifest_head,
        sections = manifest_sections,
        gaiji = manifest_gaiji,
        cover_spine = cover_spine,
        spine_head = spine_head,
        spine = spine_sections,
        progression = progression,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_nav(
    metadata: &EpubMetadata,
    sections: &[EpubSection],
    _vertical: bool,
    title: &str,
    chapters: &[NavChapter],
    toc_vertical: bool,
    toc_page: bool,
    nav_nest: bool,
    title_toc: bool,
    cover_page: bool,
    cover_toc: bool,
) -> String {
    let mut nav_items = render_nav_items(chapters, sections, nav_nest, title_toc, title);
    if cover_toc {
        // Java xhtml_nav.vm: 表紙ページへの項目を目次の先頭に追加する。
        nav_items = format!(
            "\t\t\t<li class=\"chapter\" id=\"toccover\"><a href=\"xhtml/cover.xhtml\">表紙</a></li>\r\n{nav_items}"
        );
    }
    if chapters.is_empty() && title_toc && sections.iter().any(is_title_page) {
        // Java: タイトルページを目次の先頭に書籍タイトルで追加する
        let title = xml_escape(&metadata.title);
        nav_items =
            format!("\t\t\t<li><a href=\"xhtml/title.xhtml\">{title}</a>\r\n</li>\r\n{nav_items}");
    }
    let toc_style = if toc_vertical {
        "@page {margin:.5em .5em 0 0;}\r\nhtml {\r\n\twriting-mode: vertical-rl;\r\n\t-webkit-writing-mode: vertical-rl;\r\n\t-epub-writing-mode: vertical-rl;\r\n}\r\nh1 {font-size:1.5em; padding-top:1em;}\r\nli {padding:0 .25em 0 0;}\r\nli a {text-decoration:none; border-right-width:1px; border-right-style:solid; padding-right: 1px;}\r\n.tcy {\r\n  -webkit-text-combine:         horizontal;\r\n  -webkit-text-combine-upright: all;\r\n  text-combine-upright:         all;\r\n  -epub-text-combine:           horizontal;\r\n}\r\n.upr {\r\ntext-orientation: upright;\r\n-webkit-text-orientation: upright;\r\n-epub-text-orientation: upright;\r\n}"
    } else {
        "@page {margin:.5em 0 0 .5em;}\r\nhtml {\r\n\twriting-mode:horizontal-tb;\r\n\t-webkit-writing-mode:horizontal-tb;\r\n\t-epub-writing-mode:horizontal-tb;\r\n}\r\nh1 {font-size:1.5em; text-align:center;}\r\nli {padding:.25em 0 0 0;}\r\nli a {text-decoration:none; border-bottom-width:1px; border-bottom-style:solid; padding-right: 1px;}"
    };
    let first_body = sections
        .iter()
        .enumerate()
        .find(|(_, section)| !is_title_page(section))
        .map(|(index, _)| {
            let body_number = sections[..=index]
                .iter()
                .filter(|section| !is_title_page(section))
                .count();
            format!("xhtml/{body_number:04}.xhtml")
        });
    let mut landmark = String::new();
    if cover_page {
        // Java xhtml_nav.vm landmarks: 表紙ページへの項目。
        landmark.push_str(
            "\t\t\t<li><a epub:type=\"cover\" href=\"xhtml/cover.xhtml\">表紙</a></li>\r\n",
        );
    }
    if toc_page {
        landmark.push_str("\t\t\t<li><a epub:type=\"toc\" href=\"nav.xhtml\">目次</a></li>\r\n");
    }
    if sections.iter().any(is_title_page) {
        landmark.push_str(
            "\t\t\t<li><a epub:type=\"titlepage\" href=\"xhtml/title.xhtml\">扉</a></li>\r\n",
        );
    }
    if let Some(path) = first_body {
        landmark.push_str(&format!(
            "\t\t\t<li><a epub:type=\"bodymatter\" href=\"{path}\">本文</a></li>\r\n"
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<!DOCTYPE html>\r\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"ja\" xml:lang=\"ja\">\r\n<head>\r\n<meta charset=\"UTF-8\"/>\r\n<title>{title}</title>\r\n<style type=\"text/css\">\r\n{toc_style}\r\nli {{list-style:none;}}\r\nli.chapter {{list-style:disc; line-height:1.75em;}}\r\nnav#landmarks {{ display:none; }}\r\n</style>\r\n</head>\r\n\r\n<body>\r\n\t<nav epub:type=\"landmarks\" id=\"landmarks\" hidden=\"\">\r\n\t\t<h2>Guide</h2>\r\n\t\t<ol>\r\n{landmark}\t\t</ol>\r\n\t</nav>\r\n\t<nav epub:type=\"toc\" id=\"toc\">\r\n\t\t<h1>目　次</h1>\r\n\t\t<ol>\r\n{items}\t\t</ol>\r\n\t</nav>\r\n</body>\r\n</html>\r\n",
        title = xml_escape(&metadata.title),
        toc_style = toc_style,
        landmark = landmark,
        items = nav_items,
    )
}

fn render_nav_items(
    chapters: &[NavChapter],
    sections: &[EpubSection],
    nav_nest: bool,
    title_toc: bool,
    title: &str,
) -> String {
    if chapters.is_empty() {
        // Java: 章情報が無い場合は最初の本文セクションを「本文」で出力する
        let fallback = first_body_path(sections);
        return format!("\t\t\t<li><a href=\"{fallback}\">本文</a></li>\r\n\r\n");
    }
    let mut entries: Vec<TocEntry> = chapters
        .iter()
        .map(|chapter| {
            let anchor = chapter
                .anchor
                .as_deref()
                .map(|anchor| format!("#{anchor}"))
                .unwrap_or_default();
            TocEntry {
                label: chapter.label.clone(),
                markup: chapter.markup,
                path: format!("{}{}", chapter.path, anchor),
                level: chapter.level as usize,
                level_start: 0,
                level_end: 0,
                nav_close: 0,
            }
        })
        .collect();
    // Java insertTitleToc: the title page joins the TOC as the first entry.
    if title_toc && sections.iter().any(is_title_page) {
        // Java: 目次の表題項目は bookInfo.title（生の表題文字列）を使う。
        entries.insert(
            0,
            TocEntry {
                label: title.to_owned(),
                markup: false,
                path: "xhtml/title.xhtml".to_owned(),
                level: 0,
                level_start: 0,
                level_end: 0,
                nav_close: 0,
            },
        );
    }
    set_toc_nest_level(&mut entries, false, title_toc);
    let mut output = String::new();
    for (index, entry) in entries.iter().enumerate() {
        // Java xhtml_nav.vm: close the previous <li> unless this entry opens
        // a nested list (levelStart) or nesting is off and it is not first.
        if (entry.level_start == 0 && index > 0) || (!nav_nest && index != 0) {
            output.push_str("</li>\r\n");
        }
        if nav_nest {
            for _ in 0..entry.level_start {
                output.push_str("\t\t<ol>\r\n");
            }
        }
        let label = if entry.markup {
            entry.label.clone()
        } else {
            xml_escape(&entry.label)
        };
        output.push_str(&format!(
            "\t\t\t<li><a href=\"{}\">{label}</a>\r\n",
            entry.path,
        ));
        if nav_nest {
            for _ in 0..entry.level_end {
                output.push_str("\t\t</li></ol>\r\n");
            }
        }
    }
    output.push_str("\r\n\t\t</li>\r\n");
    output
}
fn first_body_path(sections: &[EpubSection]) -> String {
    sections
        .iter()
        .enumerate()
        .find(|(_, section)| !is_title_page(section))
        .map(|(index, _)| {
            let body_number = sections[..=index]
                .iter()
                .filter(|section| !is_title_page(section))
                .count();
            format!("xhtml/{body_number:04}.xhtml")
        })
        .unwrap_or_else(|| "xhtml/0001.xhtml".to_owned())
}
/// Java toc.ncx.vm の `#if (!$hasNcxItem)` フォールバック。章情報も表紙目次も
/// 無いときは最初のセクションだけをインデント無しの navPoint で出力する。
fn render_ncx_fallback(metadata: &EpubMetadata, sections: &[EpubSection]) -> String {
    let identifier = metadata
        .identifier
        .strip_prefix("urn:uuid:")
        .or_else(|| metadata.identifier.strip_prefix("urn:"))
        .unwrap_or(&metadata.identifier);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
<head>
<meta name="dtb:uid" content="urn:uuid:{identifier}"/>
<meta name="dtb:depth" content="1"/>
<meta name="dtb:totalPageCount" content="0"/>
<meta name="dtb:maxPageNumber" content="0"/>
</head>
<docTitle>
	<text>{title}</text>
</docTitle>
<navMap>
<navPoint id="toc1" playOrder="1">
<navLabel>
<text>本文</text>
</navLabel>
<content src="{path}"/>
</navPoint>
</navMap>
</ncx>
"#,
        identifier = xml_escape(identifier),
        title = xml_escape(&metadata.title),
        path = xml_escape(&first_body_path(sections)),
    )
}

pub(super) fn render_ncx(
    metadata: &EpubMetadata,
    sections: &[EpubSection],
    title: &str,
    chapters: &[NavChapter],
    ncx_nest: bool,
    title_toc: bool,
    cover_toc: bool,
) -> String {
    // Java toc.ncx.vm: toccover が playOrder 1 を占めるため hasNcxItem が真になり、
    // 本文のみのフォールバックも使われない。
    let mut entries: Vec<TocEntry> = if chapters.is_empty() && !cover_toc {
        // Java toc.ncx.vm: 章情報も表紙目次も無いときは `#if (!$hasNcxItem)` の
        // フォールバックに入り、最初のセクションだけをインデント無しの
        // navPoint で出力して `#break` する。
        return render_ncx_fallback(metadata, sections);
    } else {
        chapters
            .iter()
            .map(|chapter| {
                let anchor = chapter
                    .anchor
                    .as_deref()
                    .map(|anchor| format!("#{anchor}"))
                    .unwrap_or_default();
                TocEntry {
                    label: chapter.label.clone(),
                    markup: chapter.markup,
                    path: format!("{}{}", chapter.path, anchor),
                    level: chapter.level as usize,
                    level_start: 0,
                    level_end: 0,
                    nav_close: 0,
                }
            })
            .collect()
    };
    // Java insertTitleToc: the title page joins the TOC as the first entry.
    if title_toc && sections.iter().any(is_title_page) {
        entries.insert(
            0,
            TocEntry {
                label: title.to_owned(),
                markup: false,
                path: "xhtml/title.xhtml".to_owned(),
                level: 0,
                level_start: 0,
                level_end: 0,
                nav_close: 0,
            },
        );
    }
    set_toc_nest_level(&mut entries, ncx_nest, title_toc);
    let identifier = metadata
        .identifier
        .strip_prefix("urn:uuid:")
        .or_else(|| metadata.identifier.strip_prefix("urn:"))
        .unwrap_or(&metadata.identifier);
    let mut nav_points = String::new();
    if cover_toc {
        nav_points.push_str(
            "\t<navPoint id=\"toccover\" playOrder=\"1\">\n\
            \t\t<navLabel>\n\
            \t\t\t<text>表紙</text>\n\
            \t\t</navLabel>\n\
            \t\t<content src=\"xhtml/cover.xhtml\"/>\n\
            \t</navPoint>\n",
        );
    }
    // 表紙項目があると以降の playOrder が 1 ずれる。
    let play_order_offset = usize::from(cover_toc);
    for (index, entry) in entries.iter().enumerate() {
        let play_order = index + 1 + play_order_offset;
        let label = if entry.markup {
            entry.label.clone()
        } else {
            xml_escape(&entry.label)
        };
        nav_points.push_str(&format!(
            "\t<navPoint id=\"toc{play_order}\" playOrder=\"{play_order}\">\n\
            \t\t<navLabel>\n\
            \t\t\t<text>{label}</text>\n\
            \t\t</navLabel>\n\
            \t\t<content src=\"{}\"/>\n",
            xml_escape(&entry.path),
        ));
        // Java toc.ncx.vm: navClose navPoint closes follow the content. With
        // ncxNest off every entry self-closes (navClose = 1); with it on,
        // parents stay open until their last descendant.
        let closes = if ncx_nest { entry.nav_close } else { 1 };
        for _ in 0..closes {
            nav_points.push_str("\t</navPoint>\n");
        }
    }
    let depth = if ncx_nest {
        entries
            .iter()
            .map(|entry| entry.level + 1)
            .max()
            .unwrap_or(1)
    } else {
        1
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
<head>
<meta name="dtb:uid" content="urn:uuid:{identifier}"/>
<meta name="dtb:depth" content="{depth}"/>
<meta name="dtb:totalPageCount" content="0"/>
<meta name="dtb:maxPageNumber" content="0"/>
</head>
<docTitle>
	<text>{title}</text>
</docTitle>
<navMap>
{nav_points}</navMap>
</ncx>
"#,
        identifier = xml_escape(identifier),
        title = xml_escape(&metadata.title),
        depth = depth,
        nav_points = nav_points,
    )
}

/// Java `template/item/xhtml/cover.vm`: 固定レイアウトの表紙ページ。
/// `svgCoverImage` 分岐（タイトル・著者を描く SVG 表紙）は GUI の
/// 確認ダイアログからのみ設定されるため対象外。
pub(super) fn render_cover(
    metadata: &EpubMetadata,
    asset_path: &str,
    dimensions: Option<(u32, u32)>,
    kindle: bool,
) -> String {
    let _ = kindle;
    let language = xml_escape(&metadata.language);
    let title = xml_escape(&metadata.title);
    let image_name = asset_path.rsplit('/').next().unwrap_or(asset_path);
    let (width, height) = dimensions.unwrap_or((0, 0));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html
xmlns="http://www.w3.org/1999/xhtml"
xmlns:epub="http://www.idpf.org/2007/ops"
xml:lang="{language}"
>
<head>
<meta charset="UTF-8"/>
<title>{title}</title>
<link rel="stylesheet" type="text/css" href="../style/fixed-layout-jp.css"/>
<meta name="viewport" content="width={width}, height={height}"/>
</head>
<body epub:type="cover">
<div class="main">
<svg xmlns="http://www.w3.org/2000/svg" version="1.1"
xmlns:xlink="http://www.w3.org/1999/xlink"
width="100%" height="100%" viewBox="0 0 {width} {height}">
<image width="{width}" height="{height}" xlink:href="../image/{image_name}"/>
</svg>
</div>
</body>
</html>"#
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn render_section(
    metadata: &EpubMetadata,
    body_fragment: &str,
    vertical: bool,
    kindle: bool,
    title_markup: Option<&str>,
    creator_markup: Option<&str>,
    title_page_markup: Option<&str>,
    title_page_type: usize,
) -> String {
    let kindle_class = if kindle { " kindle" } else { "" };
    let trimmed = body_fragment.trim();
    // Java TITLE_HORIZONTAL (TitlePage=2): title_horizontal.vm — 横書き専用の
    // 簡易表題ページ。custom markup は使わず TITLE/CREATOR 等を直接埋め込む。
    if trimmed == TITLE_PAGE_MARKER && title_page_type == 2 {
        let title = title_markup
            .map(str::to_owned)
            .unwrap_or_else(|| xml_escape(&metadata.title));
        let creator = creator_markup
            .map(str::to_owned)
            .or_else(|| metadata.creator.as_deref().map(xml_escape));
        let publisher_block = metadata
            .publisher
            .as_deref()
            .map(|value| {
                format!(
                    "<div class=\"label\">\n<p class=\"label-name\">{}</p>\n</div>\n",
                    xml_escape(value)
                )
            })
            .unwrap_or_default();
        let creator_block = creator
            .map(|value| format!("<p>{value}</p>\n"))
            .unwrap_or_default();
        return format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<!DOCTYPE html>\r\n<html\r\nxmlns=\"http://www.w3.org/1999/xhtml\"\r\nxmlns:epub=\"http://www.idpf.org/2007/ops\"\r\nxml:lang=\"{language}\"\r\nclass=\"hltr\"\r\n>\r\n<head>\r\n<meta charset=\"UTF-8\"/>\r\n<title>{title_text}</title>\r\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../style/book-style.css\"/>\r\n</head>\r\n<body class=\"p-titlepage{kindle_class}\">\r\n<div class=\"main\">\r\n\r\n<div class=\"book-title\">\r\n<div class=\"book-title-main\">\r\n<p>{title}</p>\r\n</div>\r\n</div>\r\n\r\n<div class=\"author\">\r\n{creator_block}</div>\r\n{publisher_block}</div>\r\n</body>\r\n</html>\r\n\r\n",
            language = xml_escape(&metadata.language),
            title_text = xml_escape(&metadata.title),
            title = title,
            creator_block = creator_block,
            publisher_block = publisher_block,
            kindle_class = kindle_class,
        );
    }
    if trimmed == TITLE_PAGE_MARKER {
        let custom_title_page = title_page_markup.is_some();
        let publisher = metadata
            .publisher
            .as_deref()
            .map(|value| {
                if custom_title_page {
                    format!(
                        "\t<div class=\"publisher\">{}</div>\n\t<br/>\n",
                        xml_escape(value)
                    )
                } else {
                    format!(
                        "\n<div class=\"publisher\"><p>{}</p></div>",
                        xml_escape(value)
                    )
                }
            })
            .unwrap_or_default();
        let creator_break_count = if let Some(markup) = title_page_markup {
            markup.matches("class=\"creator ").count()
                + markup.matches("class=\"subcreator ").count()
        } else if creator_markup.is_some() || metadata.creator.is_some() {
            1
        } else {
            0
        };
        let mut title_page_body = String::from("\n");
        title_page_body.push_str(&publisher);
        for index in 0..creator_break_count {
            if index == 0 {
                title_page_body.push_str("\n\t<br/>\n");
            } else {
                title_page_body.push_str("\t<br/>\n");
            }
        }
        title_page_body.push('\n');
        if let Some(markup) = title_page_markup {
            title_page_body.push_str(markup);
        } else {
            title_page_body.push_str("<div class=\"book-title start-2em\">\n");
            title_page_body.push_str("\t<div class=\"title book-title-main\"><p>");
            if let Some(title_markup) = title_markup {
                title_page_body.push_str(title_markup);
            } else {
                title_page_body.push_str(&xml_escape(&metadata.title));
            }
            title_page_body.push_str("</p></div>\n</div>");
            if let Some(creator_markup) = creator_markup {
                title_page_body.push_str(&format!(
                    "\n<div class=\"author\"><p>{creator_markup}</p></div>"
                ));
            } else if let Some(creator) = metadata.creator.as_deref() {
                title_page_body.push_str(&format!(
                    "\n<div class=\"author\"><p>{}</p></div>",
                    xml_escape(creator)
                ));
            }
        }
        title_page_body.push_str("\n\n");
        let layout_class = if vertical { "hltr" } else { "vrtl" };
        return format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<!DOCTYPE html>\r\n<html\r\n xmlns=\"http://www.w3.org/1999/xhtml\"\r\n xmlns:epub=\"http://www.idpf.org/2007/ops\"\r\n xml:lang=\"{language}\"\r\n class=\"{layout_class}\"\r\n>\r\n<head>\r\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../style/book-style.css\"/>\r\n\r\n<title>{title_text}</title>\r\n</head>\r\n\r\n\r\n<body class=\"p-titlepage{kindle_class}\">\r\n<div class=\"main vrtl block-align-center\">{title_page_body}</div>\r\n</body>\r\n</html>\r\n",
            language = xml_escape(&metadata.language),
            title_text = xml_escape(&metadata.title),
            kindle_class = kindle_class,
        );
    }

    let (page_class, raw_body_fragment) = section_page_mode(trimmed);
    let body_fragment = dedent_fragment(&sanitize_xhtml_fragment(raw_body_fragment));
    if let Some(image) = image_page_body(&body_fragment) {
        return format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<!DOCTYPE html>\r\n<html\r\n xmlns=\"http://www.w3.org/1999/xhtml\"\r\n xmlns:epub=\"http://www.idpf.org/2007/ops\"\r\n xml:lang=\"{language}\"\r\n class=\"hltr\"\r\n>\r\n<head>\r\n<meta charset=\"UTF-8\"/>\r\n<title>{title}</title>\r\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../style/book-style.css\"/>\r\n\r\n</head>\r\n<body class=\"p-image{kindle_class}\">\r\n<div class=\"main\">\r\n{image}\n</div>\r\n</body>\r\n</html>\r\n",
            language = xml_escape(&metadata.language),
            title = xml_escape(&metadata.title),
            image = image,
            kindle_class = kindle_class,
        );
    }
    if let Some(svg) = svg_image_body(&body_fragment) {
        let (width, height) = svg_view_box(svg).unwrap_or((1, 1));
        return format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<!DOCTYPE html>\r\n<html\r\nxmlns=\"http://www.w3.org/1999/xhtml\"\r\nxmlns:epub=\"http://www.idpf.org/2007/ops\"\r\nxml:lang=\"{language}\"\r\n>\r\n<head>\r\n<meta charset=\"UTF-8\"/>\r\n<title>{title}</title>\r\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../style/fixed-layout-jp.css\"/>\r\n<meta name=\"viewport\" content=\"width={width}, height={height}\"/>\r\n</head>\r\n<body>\r\n<div class=\"main\">\r\n{svg}\n</div>\r\n</body>\r\n</html>",
            language = xml_escape(&metadata.language),
            title = xml_escape(&metadata.title),
            width = width,
            height = height,
            svg = svg,
        );
    }

    let page_text = page_class.contains("p-middle") || page_class.contains("p-bottom");
    let layout_class = if page_class.contains("p-middle") {
        "hltr"
    } else if page_class.contains("p-bottom") || vertical {
        "vrtl"
    } else {
        "hltr"
    };
    let rendered_page_class = if page_text {
        format!(" class=\"p-text{kindle_class}\"")
    } else {
        body_class("", kindle)
    };
    let body = if page_class.contains("p-middle") {
        format!(
            "<div class=\"main vrtl block-align-center\">\n<div class=\"start-2em\">\n{body_fragment}\n</div>\n</div>"
        )
    } else if page_class.contains("p-bottom") {
        format!(
            "<div class=\"main vrtl block-align-end\">\n<div class=\"start-2em\">\n{body_fragment}\n</div>\n</div>"
        )
    } else {
        format!("<div class=\"main\">\n{body_fragment}\n</div>")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<!DOCTYPE html>\r\n<html\r\n xmlns=\"http://www.w3.org/1999/xhtml\"\r\n xmlns:epub=\"http://www.idpf.org/2007/ops\"\r\n xml:lang=\"{language}\"\r\n class=\"{layout_class}\"\r\n>\r\n<head>\r\n<meta charset=\"UTF-8\"/>\r\n<title>{title}</title>\r\n<link rel=\"stylesheet\" type=\"text/css\" href=\"../style/book-style.css\"/>\r\n\r\n</head>\r\n<body{rendered_page_class}>\r\n{body}\n</body>\r\n</html>\r\n",
        language = xml_escape(&metadata.language),
        title = xml_escape(&metadata.title),
        layout_class = layout_class,
        rendered_page_class = rendered_page_class,
        body = body,
    )
}

fn body_class(page_class: &str, kindle: bool) -> String {
    let kindle_class = if kindle { " kindle" } else { "" };
    if page_class.is_empty() {
        if kindle {
            " class=\"kindle\"".to_owned()
        } else {
            String::new()
        }
    } else {
        format!(
            " class=\"{}{}\"",
            page_class
                .trim_start()
                .strip_prefix("class=\"")
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(page_class.trim()),
            kindle_class
        )
    }
}

fn dedent_fragment(fragment: &str) -> String {
    fragment
        .lines()
        .map(|line| line.strip_prefix("    ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
        + if fragment.ends_with('\n') { "\n" } else { "" }
}

fn sanitize_xhtml_fragment(fragment: &str) -> String {
    let mut output = String::with_capacity(fragment.len());
    let mut cursor = 0;
    let mut paragraph_depth = 0usize;
    while let Some(relative_start) = fragment[cursor..].find('<') {
        let start = cursor + relative_start;
        output.push_str(&fragment[cursor..start]);
        let Some(relative_end) = fragment[start..].find('>') else {
            output.push_str(&fragment[start..]);
            return output;
        };
        let end = start + relative_end + 1;
        let tag = &fragment[start..end];
        let (closing, name) = tag_name(tag);
        if name == Some("p") {
            if closing {
                paragraph_depth = paragraph_depth.saturating_sub(1);
            }
            output.push_str(tag);
            if !closing {
                paragraph_depth += 1;
            }
        } else if paragraph_depth > 0
            && name
                .is_some_and(|name| matches!(name, "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"))
        {
            output.push_str(&replace_tag_name(tag, "span"));
        } else {
            output.push_str(tag);
        }
        cursor = end;
    }
    output.push_str(&fragment[cursor..]);
    output
}

fn tag_name(tag: &str) -> (bool, Option<&str>) {
    let bytes = tag.as_bytes();
    let closing = bytes.get(1) == Some(&b'/');
    let mut start = if closing { 2 } else { 1 };
    while bytes.get(start).is_some_and(u8::is_ascii_whitespace) {
        start += 1;
    }
    let end = (start..bytes.len())
        .find(|&index| !bytes[index].is_ascii_alphanumeric())
        .unwrap_or(bytes.len());
    (closing, (end > start).then(|| &tag[start..end]))
}

fn svg_image_body(body: &str) -> Option<&str> {
    let body = body.trim();
    (body.starts_with("<svg ") && body.ends_with("</svg>")).then_some(body)
}
pub(super) fn is_image_only(sections: &[EpubSection]) -> bool {
    !sections.is_empty()
        && sections
            .iter()
            .all(|section| svg_image_body(section.body_fragment.trim()).is_some())
}

fn svg_view_box(svg: &str) -> Option<(u32, u32)> {
    let marker = "viewBox=\"0 0 ";
    let start = svg.find(marker)? + marker.len();
    let value = &svg[start..];
    let end = value.find('"')?;
    let mut parts = value[..end].split_whitespace();
    let width = parts.next()?.parse().ok()?;
    let height = parts.next()?.parse().ok()?;
    Some((width, height))
}

fn replace_tag_name(tag: &str, replacement: &str) -> String {
    let closing = tag.as_bytes().get(1) == Some(&b'/');
    let start = if closing { 2 } else { 1 };
    let end = (start..tag.len())
        .find(|&index| !tag.as_bytes()[index].is_ascii_alphanumeric())
        .unwrap_or(tag.len());
    format!("{}{}{}", &tag[..start], replacement, &tag[end..])
}

fn section_page_mode(body: &str) -> (&'static str, &str) {
    let body = body
        .strip_prefix(PAGE_CHAPTER_MARKER)
        .map_or(body, str::trim_start);
    if let Some(body) = body.strip_prefix(PAGE_MIDDLE_MARKER) {
        let body = body
            .strip_prefix(PAGE_CHAPTER_MARKER)
            .map_or(body, str::trim_start);
        return (" class=\"p-middle\"", body.trim());
    }
    if let Some(body) = body.strip_prefix(PAGE_BOTTOM_MARKER) {
        let body = body
            .strip_prefix(PAGE_CHAPTER_MARKER)
            .map_or(body, str::trim_start);
        return (" class=\"p-bottom\"", body.trim());
    }
    if let Some(body) = body.strip_prefix(PAGE_NO_CHAPTER_MARKER) {
        return ("", body.trim());
    }
    ("", body)
}

fn image_page_body(body: &str) -> Option<&str> {
    // Java は単ページ画像セクションのみ <span><img class="fit"> を p 無しで出力し
    // <html class="hltr"> + <body class="p-image"> にする。本文中の fit は p 内 span。
    // 単ページ画像セクションの p 除去は split_image_page_sections 側で行う。
    let body = body.trim();
    if body.starts_with("<span><img class=\"fit\"") && body.ends_with("</span>") {
        return Some(body);
    }
    let inner = body.strip_prefix("<p>")?.strip_suffix("</p>")?.trim();
    (inner.starts_with("<img class=\"fit\"") && inner.ends_with("/>")).then_some(body)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
