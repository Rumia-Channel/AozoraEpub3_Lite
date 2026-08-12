use super::{EpubAsset, EpubMetadata, EpubSection, TITLE_PAGE_MARKER, is_title_page};
const PAGE_MIDDLE_MARKER: &str = "<!-- aozora-page-middle -->";
const PAGE_BOTTOM_MARKER: &str = "<!-- aozora-page-bottom -->";
const PAGE_NO_CHAPTER_MARKER: &str = "<!-- aozora-page-no-chapter -->";

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
    path: String,
    level: usize,
}
fn is_no_chapter(section: &EpubSection) -> bool {
    section
        .body_fragment
        .trim_start()
        .starts_with(PAGE_NO_CHAPTER_MARKER)
}

fn nav_entries(sections: &[EpubSection]) -> Vec<NavEntry> {
    let body_count = sections
        .iter()
        .filter(|section| !is_title_page(section) && !is_no_chapter(section))
        .count();
    let mut body_number = 0;
    let mut entries = Vec::with_capacity(sections.len());
    for section in sections {
        if !is_title_page(section) {
            body_number += 1;
        }
        if is_no_chapter(section) {
            continue;
        }
        let level = if is_title_page(section) {
            1
        } else {
            first_heading(section.body_fragment.as_str())
                .map(|(level, _)| level)
                .unwrap_or(1)
        };
        entries.push(NavEntry {
            label: section_label(section, body_number, body_count),
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
    first_heading_label(&section.body_fragment).unwrap_or_else(|| {
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
        let label = strip_html(&body[content_start..content_end]);
        if !label.trim().is_empty() {
            return Some((level + 1, label.trim().to_owned()));
        }
    }
    None
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

pub(super) fn render_package(
    metadata: &EpubMetadata,
    sections: &[EpubSection],
    assets: &[EpubAsset],
    cover_asset: Option<&str>,
    vertical: bool,
) -> String {
    let creator = metadata
        .creator
        .as_deref()
        .map(|value| {
            format!(
                "\n    <dc:creator id=\"creator\">{}</dc:creator>",
                xml_escape(value)
            )
        })
        .unwrap_or_default();
    let publisher = metadata
        .publisher
        .as_deref()
        .map(|value| format!("\n    <dc:publisher>{}</dc:publisher>", xml_escape(value)))
        .unwrap_or_default();
    let mut body_number = 0;
    let mut manifest_sections = String::new();
    let mut spine_sections = String::new();
    for section in sections {
        if is_title_page(section) {
            manifest_sections.push_str(
                "    <item id=\"title\" href=\"xhtml/title.xhtml\" media-type=\"application/xhtml+xml\"/>\n",
            );
            spine_sections.push_str("    <itemref idref=\"title\"/>\n");
        } else {
            body_number += 1;
            let properties = if svg_image_body(section.body_fragment.trim()).is_some() {
                " properties=\"svg\""
            } else {
                ""
            };
            manifest_sections.push_str(&format!(
                "    <item id=\"section-{body_number:04}\" href=\"xhtml/{body_number:04}.xhtml\" media-type=\"application/xhtml+xml\"{properties}/>\n"
            ));
            spine_sections.push_str(&format!(
                "    <itemref idref=\"section-{body_number:04}\"/>\n"
            ));
        }
    }
    let mut manifest_assets = String::new();
    for (index, asset) in assets.iter().enumerate() {
        let properties = if cover_asset == Some(asset.path.as_str()) {
            " properties=\"cover-image\""
        } else {
            ""
        };
        manifest_assets.push_str(&format!(
            "    <item id=\"asset-{index:04}\" href=\"{}\" media-type=\"{}\"{properties}/>\n",
            xml_escape(&asset.path),
            xml_escape(&asset.media_type),
        ));
    }
    let cover_manifest = if cover_asset.is_some() {
        "    <item id=\"cover\" href=\"cover.xhtml\" media-type=\"application/xhtml+xml\"/>\n"
    } else {
        ""
    };
    let cover_spine = if cover_asset.is_some() {
        "    <itemref idref=\"cover\"/>\n"
    } else {
        ""
    };

    let progression = if vertical { "rtl" } else { "ltr" };

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>{}</dc:title>{}{}
    <dc:language>{}</dc:language>
    <dc:identifier id="pub-id">{}</dc:identifier>
    <meta property="dcterms:modified">{}</meta>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="style" href="style/book-style.css" media-type="text/css"/>
    <item id="style-reset" href="style/style-reset.css" media-type="text/css"/>
    <item id="style-standard" href="style/style-standard.css" media-type="text/css"/>
    <item id="style-advance" href="style/style-advance.css" media-type="text/css"/>
    <item id="aozora-style" href="style/aozora.css" media-type="text/css"/>
    <item id="font-style" href="style/font.css" media-type="text/css"/>
    <item id="text-style" href="style/text.css" media-type="text/css"/>
    <item id="fixed-layout-style" href="style/fixed-layout-jp.css" media-type="text/css"/>
{}{}{}    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine page-progression-direction="{progression}" toc="ncx">
{}{}  </spine>
</package>
"#,
        xml_escape(&metadata.title),
        creator,
        publisher,
        xml_escape(&metadata.language),
        xml_escape(&metadata.identifier),
        xml_escape(&metadata.modified),
        manifest_sections,
        manifest_assets,
        cover_manifest,
        cover_spine,
        spine_sections,
        progression = progression,
    )
}

pub(super) fn render_nav(metadata: &EpubMetadata, sections: &[EpubSection]) -> String {
    let nav_items = render_nav_items(&nav_entries(sections));

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{language}">
<head>
  <meta charset="UTF-8"/>
  <title>{title}</title>
</head>
<body>
  <nav epub:type="toc" id="toc">
    <h1>目次</h1>
    <ol>
{items}    </ol>
  </nav>
</body>
</html>
"#,
        language = xml_escape(&metadata.language),
        title = xml_escape(&metadata.title),
        items = nav_items,
    )
}

fn render_nav_items(entries: &[NavEntry]) -> String {
    let mut output = String::new();
    let mut depth = 1usize;
    let mut has_item = false;
    for entry in entries {
        let level = entry.level;
        if has_item {
            if level > depth {
                for _ in depth..level {
                    output.push_str("      <ol>\n");
                }
            } else {
                output.push_str("      </li>\n");
                for _ in level..depth {
                    output.push_str("      </ol>\n      </li>\n");
                }
            }
        }
        output.push_str(&format!(
            "      <li><a href=\"{}\">{}</a>",
            xml_escape(&entry.path),
            xml_escape(&entry.label),
        ));
        depth = level;
        has_item = true;
        output.push('\n');
    }
    if has_item {
        output.push_str("      </li>\n");
        for _ in 1..depth {
            output.push_str("      </ol>\n      </li>\n");
        }
    }
    output
}

pub(super) fn render_ncx(metadata: &EpubMetadata, sections: &[EpubSection]) -> String {
    let entries = nav_entries(sections);
    let mut nav_points = String::new();
    let mut current_depth = 0usize;
    for (index, entry) in entries.iter().enumerate() {
        while current_depth >= entry.level && current_depth > 0 {
            let indent = "    ".repeat(current_depth);
            nav_points.push_str(&format!("{indent}</navPoint>\n"));
            current_depth -= 1;
        }
        let indent = "    ".repeat(entry.level);
        let play_order = index + 1;
        nav_points.push_str(&format!(
            "{indent}<navPoint id=\"navpoint-{play_order}\" playOrder=\"{play_order}\">\n\
{indent}  <navLabel><text>{}</text></navLabel>\n\
{indent}  <content src=\"{}\"/>\n",
            xml_escape(&entry.label),
            xml_escape(&entry.path),
        ));
        current_depth = entry.level;
    }
    while current_depth > 0 {
        let indent = "    ".repeat(current_depth);
        nav_points.push_str(&format!("{indent}</navPoint>\n"));
        current_depth -= 1;
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd">
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head>
    <meta name="dtb:uid" content="{identifier}"/>
  </head>
  <docTitle><text>{title}</text></docTitle>
  <navMap>
{nav_points}  </navMap>
</ncx>
"#,
        identifier = xml_escape(&metadata.identifier),
        title = xml_escape(&metadata.title),
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
) -> String {
    let kindle_class = if kindle { " kindle" } else { "" };
    let trimmed = body_fragment.trim();
    if trimmed == TITLE_PAGE_MARKER {
        let publisher = metadata
            .publisher
            .as_deref()
            .map(|value| {
                format!(
                    "\n    <div class=\"publisher\"><p>{}</p></div>",
                    xml_escape(value)
                )
            })
            .unwrap_or_default();
        let creator = metadata
            .creator
            .as_deref()
            .map(|value| {
                format!(
                    "\n    <div class=\"author\"><p>{}</p></div>",
                    xml_escape(value)
                )
            })
            .unwrap_or_default();
        return format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{language}" class="hltr">
<head>
  <meta charset="UTF-8"/>
  <title>{title}</title>
  <link rel="stylesheet" type="text/css" href="../style/book-style.css"/>
</head>
<body class="p-titlepage{kindle_class}">
  <div class="book-title">{publisher}
    <div class="book-title-main"><p>{title}</p></div>{creator}
  </div>
</body>
</html>
"#,
            language = xml_escape(&metadata.language),
            title = xml_escape(&metadata.title),
            publisher = publisher,
            creator = creator,
            kindle_class = kindle_class,
        );
    }

    let (page_class, raw_body_fragment) = section_page_mode(trimmed);
    let body_fragment = sanitize_xhtml_fragment(raw_body_fragment);
    if let Some(image) = image_page_body(&body_fragment) {
        return format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{language}" class="hltr">
<head>
  <meta charset="UTF-8"/>
  <title>{title}</title>
  <link rel="stylesheet" type="text/css" href="../style/book-style.css"/>
</head>
<body class="p-image{kindle_class}">
  <div class="main">{image}</div>
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
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{language}">
<head>
  <meta charset="UTF-8"/>
  <title>{title}</title>
  <link rel="stylesheet" type="text/css" href="../style/fixed-layout-jp.css"/>
  <meta name="viewport" content="width={width}, height={height}"/>
</head>
<body>
  <div class="main">{svg}</div>
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
    let page_class = body_class(page_class, kindle);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="{language}" class="{layout_class}">
<head>
  <meta charset="UTF-8"/>
  <title>{title}</title>
  <link rel="stylesheet" type="text/css" href="../style/book-style.css"/>
</head>
<body{page_class}>
  <div class="main">
{body}
  </div>
</body>
</html>
"#,
        language = xml_escape(&metadata.language),
        title = xml_escape(&metadata.title),
        layout_class = layout_class,
        page_class = page_class,
        body = body_fragment,
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
    if let Some(body) = body.strip_prefix(PAGE_MIDDLE_MARKER) {
        return (" class=\"p-middle\"", body.trim());
    }
    if let Some(body) = body.strip_prefix(PAGE_BOTTOM_MARKER) {
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
