use super::{EpubAsset, EpubMetadata, EpubSection, TITLE_PAGE_MARKER, is_title_page};
const PAGE_MIDDLE_MARKER: &str = "<!-- aozora-page-middle -->";
const PAGE_BOTTOM_MARKER: &str = "<!-- aozora-page-bottom -->";
const PAGE_NO_CHAPTER_MARKER: &str = "<!-- aozora-page-no-chapter -->";
const PAGE_CHAPTER_MARKER: &str = "<!-- aozora-page-chapter -->";

pub(super) fn section_path(section: &EpubSection, body_number: usize) -> String {
    if is_title_page(section) {
        "xhtml/title.xhtml".to_owned()
    } else {
        format!("xhtml/{body_number:04}.xhtml")
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct NavEntry {
    label: String,
    markup: bool,
    path: String,
    level: usize,
}
fn is_no_chapter(section: &EpubSection) -> bool {
    section
        .body_fragment
        .trim_start()
        .starts_with(PAGE_NO_CHAPTER_MARKER)
}
fn is_separator_section(section: &EpubSection) -> bool {
    let Some(label) = first_text_label_raw(&section.body_fragment) else {
        return false;
    };
    let leading_equals = label
        .chars()
        .take_while(|character| *character == '=')
        .count();
    let trailing_equals = label
        .chars()
        .rev()
        .take_while(|character| *character == '=')
        .count();
    leading_equals >= 2 && trailing_equals >= 2
}

fn is_image_only_section(section: &EpubSection) -> bool {
    section.body_fragment.contains("<img") && strip_html(&section.body_fragment).trim().is_empty()
}

fn normalize_chapter_label(label: String) -> String {
    let trimmed = label.trim();
    let leading_equals = trimmed
        .chars()
        .take_while(|character| *character == '=')
        .count();
    let trailing_equals = trimmed
        .chars()
        .rev()
        .take_while(|character| *character == '=')
        .count();
    if leading_equals >= 2
        && trailing_equals >= 2
        && leading_equals + trailing_equals < trimmed.len()
    {
        let inner = &trimmed[leading_equals..trimmed.len() - trailing_equals];
        format!("={inner}=")
    } else {
        trimmed.to_owned()
    }
}
fn nav_entries(sections: &[EpubSection], title_markup: Option<&str>) -> Vec<NavEntry> {
    let body_count = sections
        .iter()
        .filter(|section| {
            !is_title_page(section) && !is_no_chapter(section) && !is_image_only_section(section)
        })
        .count();
    let mut body_number = 0;
    let mut entries = Vec::with_capacity(sections.len());
    let mut first_body_entry = true;
    for section in sections {
        if !is_title_page(section) {
            body_number += 1;
        }
        let is_title = is_title_page(section);
        if is_no_chapter(section) || (!is_title && is_image_only_section(section)) {
            continue;
        }
        if !is_title && first_body_entry && is_separator_section(section) {
            first_body_entry = false;
            continue;
        }
        if !is_title {
            first_body_entry = false;
        }
        let label = if is_title {
            title_markup.unwrap_or("タイトル").to_owned()
        } else {
            section_label(section, body_number, body_count)
        };
        let markup = title_markup.is_some() && is_title;
        let level = if is_title {
            1
        } else {
            first_heading(section.body_fragment.as_str())
                .map(|(level, _)| level)
                .unwrap_or(1)
        };
        entries.push(NavEntry {
            label,
            markup,
            path: section_path(section, body_number),
            level: level.clamp(1, 3),
        });
    }
    entries
}

pub(super) fn section_label(
    section: &EpubSection,
    body_number: usize,
    body_count: usize,
) -> String {
    if is_title_page(section) {
        return "タイトル".to_owned();
    }
    first_heading_label(&section.body_fragment)
        .or_else(|| first_text_label(&section.body_fragment))
        .unwrap_or_else(|| {
            if body_count == 1 {
                "本文".to_owned()
            } else {
                format!("本文 {body_number}")
            }
        })
}

fn first_heading(body: &str) -> Option<(usize, String)> {
    for (level, element) in ["h1", "h2", "h3"].into_iter().enumerate() {
        let open = format!("<{element}");
        let Some(start) = body.find(&open) else {
            continue;
        };
        let Some(open_end) = body[start..].find('>') else {
            continue;
        };
        let content_start = start + open_end + 1;
        let close = format!("</{element}>");
        let Some(close_offset) = body[content_start..].find(&close) else {
            continue;
        };
        let content_end = content_start + close_offset;
        let label = strip_html(&strip_ruby_readings(&body[content_start..content_end]));
        if !label.trim().is_empty() {
            return Some((level + 1, normalize_chapter_label(label)));
        }
    }
    None
}

fn first_text_label_raw(body: &str) -> Option<String> {
    let mut offset = 0usize;
    while let Some(relative_start) = body[offset..].find("<p") {
        let start = offset + relative_start;
        let content_start = start + body[start..].find('>')? + 1;
        let content_end = body[content_start..].find("</p>")? + content_start;
        let label = unescape_html(&strip_html(&strip_ruby_readings(
            &body[content_start..content_end],
        )));
        if !label.trim().is_empty() {
            return Some(label.trim().to_owned());
        }
        offset = content_end + "</p>".len();
    }
    None
}

fn first_text_label(body: &str) -> Option<String> {
    first_text_label_raw(body).map(normalize_chapter_label)
}
fn unescape_html(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}
fn first_heading_label(body: &str) -> Option<String> {
    first_heading(body).map(|(_, label)| label)
}

fn strip_html(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;
    for character in input.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    output
}

fn strip_ruby_readings(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut remainder = input;
    while let Some(start) = remainder.find("<rt") {
        output.push_str(&remainder[..start]);
        let Some(open_end) = remainder[start..].find('>') else {
            output.push_str(&remainder[start..]);
            return output;
        };
        let after_open = start + open_end + 1;
        let Some(close_offset) = remainder[after_open..].find("</rt>") else {
            output.push_str(&remainder[start..]);
            return output;
        };
        remainder = &remainder[after_open + close_offset + "</rt>".len()..];
    }
    output.push_str(remainder);
    output
}

pub(super) fn render_package(
    metadata: &EpubMetadata,
    sections: &[EpubSection],
    assets: &[EpubAsset],
    cover_asset: Option<&str>,
    vertical: bool,
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
    let mut title_page_seen = false;
    for (index, section) in sections.iter().enumerate() {
        if is_title_page(section) {
            title_page_seen = true;
            manifest_sections.push_str(
                "\t\t<item id=\"title-page\" href=\"xhtml/title.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
            );
            spine_sections.push_str("\t\t<itemref idref=\"title-page\" linear=\"yes\"/>\n");
            continue;
        }
        if title_page_seen && body_number == 0 {
            manifest_sections.push('\n');
            spine_sections.push('\n');
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
    let mut manifest_assets = String::new();
    for (index, asset) in assets.iter().enumerate() {
        let properties = if !image_only && cover_asset == Some(asset.path.as_str()) {
            " properties=\"cover-image\""
        } else {
            ""
        };
        manifest_assets.push_str(&format!(
            "\t\t<item id=\"img{:04}\" href=\"{}\" media-type=\"{}\"{properties}/>\n",
            index + 1,
            xml_escape(&asset.path),
            xml_escape(&asset.media_type),
        ));
    }
    let cover_manifest = if !image_only && cover_asset.is_some() {
        "\t\t<item media-type=\"application/xhtml+xml\" id=\"cover-page\" href=\"xhtml/cover.xhtml\" properties=\"svg\"/>\n"
    } else {
        ""
    };
    let cover_spine = if !image_only && cover_asset.is_some() {
        "\t\t<itemref linear=\"yes\" idref=\"cover-page\" properties=\"rendition:page-spread-center\"/>\n"
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
{cover}{sections}
		<item href="toc.ncx" id="ncx" media-type="application/x-dtbncx+xml"/>
	</manifest>

	<spine page-progression-direction="{progression}" toc="ncx">
{cover_spine}{spine}	</spine>

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
        sections = manifest_sections.trim_end(),
        cover_spine = cover_spine,
        spine = spine_sections,
        progression = progression,
    )
}

pub(super) fn render_nav(
    metadata: &EpubMetadata,
    sections: &[EpubSection],
    vertical: bool,
    title_markup: Option<&str>,
) -> String {
    let entries = nav_entries(sections, title_markup);
    let nav_items = render_nav_items(&entries);
    let toc_style = if vertical {
        r#"@page {margin:.5em .5em 0 0;}
html {
	writing-mode: vertical-rl;
	-webkit-writing-mode: vertical-rl;
	-epub-writing-mode: vertical-rl;
}
h1 {font-size:1.5em; padding-top:1em;}
li {padding:0 .25em 0 0;}
li a {text-decoration:none; border-right-width:1px; border-right-style:solid; padding-right: 1px;}"#
    } else {
        r#"@page {margin:.5em 0 0 .5em;}
html {
	writing-mode:horizontal-tb;
	-webkit-writing-mode:horizontal-tb;
	-epub-writing-mode:horizontal-tb;
}
h1 {font-size:1.5em; text-align:center;}
li {padding:.25em 0 0 0;}
li a {text-decoration:none; border-bottom-width:1px; border-bottom-style:solid; padding-right: 1px;}"#
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
    let title_landmark = if sections.iter().any(is_title_page) {
        "\t\t\t<li><a epub:type=\"titlepage\" href=\"xhtml/title.xhtml\">扉</a></li>\n"
    } else {
        ""
    };
    let body_landmark = first_body
        .map(|path| {
            format!("\t\t\t<li><a epub:type=\"bodymatter\" href=\"{path}\">本文</a></li>\n")
        })
        .unwrap_or_default();
    let landmark = format!("{title_landmark}{body_landmark}");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" lang="{language}" xml:lang="{language}">
<head>
<meta charset="UTF-8"/>
<title>{title}</title>
<style type="text/css">
{toc_style}
.tcy {{
  -webkit-text-combine:         horizontal;
  -webkit-text-combine-upright: all;
  text-combine-upright:         all;
  -epub-text-combine:           horizontal;
}}
.upr {{
text-orientation: upright;
-webkit-text-orientation: upright;
-epub-text-orientation: upright;
}}
li {{list-style:none;}}
li.chapter {{list-style:disc; line-height:1.75em;}}
nav#landmarks {{ display:none; }}
</style>
</head>

<body>
	<nav epub:type="landmarks" id="landmarks" hidden="">
		<h2>Guide</h2>
		<ol>
{landmark}		</ol>
	</nav>
	<nav epub:type="toc" id="toc">
		<h1>目　次</h1>
		<ol>
{items}		</ol>
	</nav>
</body>
</html>
"#,
        language = xml_escape(&metadata.language),
        title = xml_escape(&metadata.title),
        toc_style = toc_style,
        landmark = landmark,
        items = nav_items,
    )
}

fn render_nav_items(entries: &[NavEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    if entries.iter().all(|entry| entry.level == 1) {
        let mut output = String::new();
        for (index, entry) in entries.iter().enumerate() {
            if index > 0 {
                output.push_str("</li>\n");
            }
            let label = if entry.markup {
                entry.label.clone()
            } else {
                xml_escape(&entry.label)
            };
            output.push_str(&format!(
                "\t\t\t<li><a href=\"{}\">{label}</a>\n",
                xml_escape(&entry.path),
            ));
        }
        output.push_str("\n\t\t</li>\n");
        return output;
    }

    let mut output = String::new();
    let mut depth = 1usize;
    let mut has_item = false;
    for entry in entries {
        let level = entry.level;
        if has_item {
            if level > depth {
                for _ in depth..level {
                    output.push_str("\t\t<ol>\n");
                }
            } else {
                output.push_str("</li>\n");
                for _ in level..depth {
                    output.push_str("</ol>\n</li>\n");
                }
            }
        }
        let label = if entry.markup {
            entry.label.clone()
        } else {
            xml_escape(&entry.label)
        };
        output.push_str(&format!(
            "\t\t\t<li><a href=\"{}\">{label}</a>\n",
            xml_escape(&entry.path),
        ));
        depth = level;
        has_item = true;
    }
    if has_item {
        output.push_str("\n\t\t</li>\n");
        for _ in 1..depth {
            output.push_str("</ol>\n</li>\n");
        }
    }
    output
}
pub(super) fn render_ncx(
    metadata: &EpubMetadata,
    sections: &[EpubSection],
    title_markup: Option<&str>,
) -> String {
    let entries = nav_entries(sections, title_markup);
    let identifier = metadata
        .identifier
        .strip_prefix("urn:uuid:")
        .or_else(|| metadata.identifier.strip_prefix("urn:"))
        .unwrap_or(&metadata.identifier);
    let mut nav_points = String::new();
    let mut current_depth = 0usize;
    for (index, entry) in entries.iter().enumerate() {
        while current_depth >= entry.level && current_depth > 0 {
            let close_indent = "\t".repeat(current_depth);
            nav_points.push_str(&format!("{close_indent}</navPoint>\n"));
            current_depth -= 1;
        }
        let indent = "\t".repeat(entry.level);
        let child_indent = format!("{indent}\t");
        let value_indent = format!("{child_indent}\t");
        let play_order = index + 1;
        let label = if entry.markup {
            entry.label.clone()
        } else {
            xml_escape(&entry.label)
        };
        nav_points.push_str(&format!(
            "{indent}<navPoint id=\"toc{play_order}\" playOrder=\"{play_order}\">\n\
        {child_indent}<navLabel>\n\
        {value_indent}<text>{label}</text>\n\
        {child_indent}</navLabel>\n\
        {child_indent}<content src=\"{}\"/>\n",
            xml_escape(&entry.path),
        ));
        current_depth = entry.level;
    }
    while current_depth > 0 {
        let close_indent = "\t".repeat(current_depth);
        nav_points.push_str(&format!("{close_indent}</navPoint>\n"));
        current_depth -= 1;
    }
    let depth = entries.iter().map(|entry| entry.level).max().unwrap_or(1);
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

pub(super) fn render_cover(metadata: &EpubMetadata, asset_path: &str, kindle: bool) -> String {
    let kindle_class = if kindle { " kindle" } else { "" };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{language}">
<head>
  <meta charset="UTF-8"/>
  <title>{title}</title>
</head>
<body class="p-image{kindle_class}">
  <div class="cover">
    <img src="{asset_path}" alt="{title}"/>
  </div>
</body>
</html>
"#,
        language = xml_escape(&metadata.language),
        title = xml_escape(&metadata.title),
        asset_path = xml_escape(asset_path),
        kindle_class = kindle_class,
    )
}

pub(super) fn render_section(
    metadata: &EpubMetadata,
    body_fragment: &str,
    vertical: bool,
    kindle: bool,
    title_markup: Option<&str>,
    creator_markup: Option<&str>,
    title_page_markup: Option<&str>,
) -> String {
    let kindle_class = if kindle { " kindle" } else { "" };
    let trimmed = body_fragment.trim();
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
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html
 xmlns="http://www.w3.org/1999/xhtml"
 xmlns:epub="http://www.idpf.org/2007/ops"
 xml:lang="{language}"
 class="{layout_class}"
>
<head>
<link rel="stylesheet" type="text/css" href="../style/book-style.css"/>

<title>{title_text}</title>
</head>


<body class="p-titlepage{kindle_class}">
<div class="main vrtl block-align-center">{title_page_body}</div>
</body>
</html>"#,
            language = xml_escape(&metadata.language),
            title_text = xml_escape(&metadata.title),
            kindle_class = kindle_class,
        );
    }

    let (page_class, raw_body_fragment) = section_page_mode(trimmed);
    let body_fragment = dedent_fragment(&sanitize_xhtml_fragment(raw_body_fragment));
    if let Some(image) = image_page_body(&body_fragment) {
        return format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html
 xmlns="http://www.w3.org/1999/xhtml"
 xmlns:epub="http://www.idpf.org/2007/ops"
 xml:lang="{language}"
 class="hltr"
>
<head>
<meta charset="UTF-8"/>
<title>{title}</title>
<link rel="stylesheet" type="text/css" href="../style/book-style.css"/>

</head>
<body class="p-image{kindle_class}">
<div class="main">
{image}
</div>
</body>
</html>
"#,
            language = xml_escape(&metadata.language),
            title = xml_escape(&metadata.title),
            image = image,
            kindle_class = kindle_class,
        );
    }
    if let Some(svg) = svg_image_body(&body_fragment) {
        let (width, height) = svg_view_box(svg).unwrap_or((1, 1));
        return format!(
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
<body>
<div class="main">
{svg}
</div>
</body>
</html>
"#,
            language = xml_escape(&metadata.language),
            title = xml_escape(&metadata.title),
            width = width,
            height = height,
            svg = svg,
        );
    }

    let layout_class = if vertical { "vrtl" } else { "hltr" };
    let page_text = page_class.contains("p-middle") || page_class.contains("p-bottom");
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
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html
 xmlns="http://www.w3.org/1999/xhtml"
 xmlns:epub="http://www.idpf.org/2007/ops"
 xml:lang="{language}"
 class="{layout_class}"
>
<head>
<meta charset="UTF-8"/>
<title>{title}</title>
<link rel="stylesheet" type="text/css" href="../style/book-style.css"/>

</head>
<body{rendered_page_class}>
{body}
</body>
</html>
"#,
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
    let body = body.strip_prefix("<p>")?.strip_suffix("</p>")?.trim();
    if body.starts_with("<img class=\"fit\"") && body.ends_with("/>") {
        return Some(body);
    }
    let image = body.strip_prefix("<span>")?.strip_suffix("</span>")?.trim();
    (image.starts_with("<img class=\"fit\"") && image.ends_with("/>")).then_some(body)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
