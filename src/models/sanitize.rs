//! Article HTML comes from a rich-text editor and is later shown on the public site AND
//! inside the admin preview, so it is sanitised on the server before it is stored.
//! Scripts, event handlers, iframes, `javascript:` / `data:` URLs and unsafe CSS are removed.

use ammonia::{Builder, UrlRelative};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

const ALLOWED_STYLE_PROPS: &[&str] = &[
    "color", "background-color", "font-family", "font-size", "font-weight", "font-style",
    "text-decoration", "text-align", "text-indent", "line-height", "letter-spacing",
    "float", "display", "margin", "margin-left", "margin-right", "margin-top", "margin-bottom",
    "padding", "padding-left", "padding-right", "width", "max-width", "height",
    "list-style-type", "vertical-align", "border", "border-collapse", "border-color",
    "border-width", "border-style",
];

pub fn clean_html(input: &str) -> String {
    let tags: HashSet<&str> = [
        "p", "br", "hr", "div", "span", "strong", "b", "em", "i", "u", "s", "strike", "sub", "sup",
        "h2", "h3", "h4", "h5", "blockquote", "pre", "code", "ul", "ol", "li", "a", "img",
        "figure", "figcaption", "table", "thead", "tbody", "tfoot", "tr", "th", "td", "caption",
    ]
    .into_iter()
    .collect();

    let generic: HashSet<&str> = ["style", "class", "dir", "lang", "title"].into_iter().collect();

    let mut tag_attrs: HashMap<&str, HashSet<&str>> = HashMap::new();
    tag_attrs.insert("a", ["href", "target"].into_iter().collect());
    tag_attrs.insert("img", ["src", "alt", "width", "height"].into_iter().collect());
    tag_attrs.insert("td", ["colspan", "rowspan"].into_iter().collect());
    tag_attrs.insert("th", ["colspan", "rowspan"].into_iter().collect());

    Builder::default()
        .tags(tags)
        .generic_attributes(generic)
        .tag_attributes(tag_attrs)
        .url_schemes(["http", "https", "mailto"].into_iter().collect())
        .url_relative(UrlRelative::PassThrough) // keeps /uploads/... image paths
        .link_rel(Some("noopener noreferrer"))
        .attribute_filter(filter_attribute)
        .clean(input)
        .to_string()
        .replace("<img>", "") // an <img> whose src was rejected is just noise
}

fn filter_attribute<'u>(_element: &str, attribute: &str, value: &'u str) -> Option<Cow<'u, str>> {
    match attribute {
        "style" => clean_style(value).map(Cow::Owned),
        "class" => {
            let ok = value.len() <= 80
                && value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_'));
            ok.then(|| Cow::Borrowed(value))
        }
        "target" => (value == "_blank").then(|| Cow::Borrowed(value)),
        "width" | "height" | "colspan" | "rowspan" => {
            let ok = !value.is_empty() && value.chars().all(|c| c.is_ascii_digit() || c == '%');
            ok.then(|| Cow::Borrowed(value))
        }
        "dir" => matches!(value, "ltr" | "rtl").then(|| Cow::Borrowed(value)),
        _ => Some(Cow::Borrowed(value)),
    }
}

/// Keeps only whitelisted CSS properties with harmless values.
fn clean_style(value: &str) -> Option<String> {
    let mut kept: Vec<String> = Vec::new();

    for decl in value.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        let Some((prop, val)) = decl.split_once(':') else { continue };
        let prop = prop.trim().to_ascii_lowercase();
        let val = val.trim();

        if !ALLOWED_STYLE_PROPS.contains(&prop.as_str()) || val.is_empty() {
            continue;
        }
        let charset_ok = val
            .chars()
            .all(|c| !c.is_ascii() || c.is_ascii_alphanumeric() || " #%.,()'\"-".contains(c));
        let lower = val.to_ascii_lowercase();
        let bad = ["url(", "expression", "javascript", "import", "behavior", "var("]
            .iter()
            .any(|b| lower.contains(b));
        if charset_ok && !bad {
            kept.push(format!("{prop}: {val}"));
        }
    }

    if kept.is_empty() {
        None
    } else {
        Some(kept.join("; "))
    }
}

/// Visible text of an HTML fragment (used for "is the body empty?" and auto-summaries).
pub fn plain_text(html: &str) -> String {
    // put a space after block-level endings so paragraphs do not run together
    let spaced = ["</p>", "</div>", "</li>", "</h2>", "</h3>", "</h4>", "</h5>", "</blockquote>", "</figcaption>", "</td>", "</th>", "<br>"]
        .iter()
        .fold(html.to_string(), |acc, t| acc.replace(t, &format!("{t} ")));
    let stripped = Builder::empty().clean(&spaced).to_string();
    stripped
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
        .replace('\u{a0}', " ")
}

/// Removes `<figure>...</figure>` blocks (photo + caption), e.g. before building a text summary.
pub fn without_figures(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find("<figure") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</figure>") {
            Some(end) => rest = &rest[start + end + "</figure>".len()..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_scripts_and_handlers() {
        let out = clean_html(r#"<p onclick="x()">Hi</p><script>alert(1)</script><iframe src="//evil"></iframe>"#);
        assert_eq!(out, "<p>Hi</p>");
    }

    #[test]
    fn keeps_safe_style_drops_unsafe() {
        let out = clean_html(r#"<span style="color: #ff0000; position: fixed; background: url(http://x/y)">a</span>"#);
        assert!(out.contains("color: #ff0000"));
        assert!(!out.contains("position"));
        assert!(!out.contains("url("));
    }

    #[test]
    fn style_with_url_value_is_removed() {
        let out = clean_html(r#"<p style="background-color: url(javascript:alert(1))">a</p>"#);
        assert_eq!(out, "<p>a</p>");
    }

    #[test]
    fn images_and_captions_survive_but_not_data_urls() {
        let ok = clean_html(r#"<figure class="image"><img src="/uploads/2026/10/a.jpg" alt="ছবি"><figcaption>ছবি: নিজস্ব প্রতিবেদক</figcaption></figure>"#);
        assert!(ok.contains("<figure") && ok.contains("<figcaption>") && ok.contains("/uploads/2026/10/a.jpg"));
        let bad = clean_html(r#"<img src="javascript:alert(1)">"#);
        assert!(!bad.contains("javascript"));
    }

    #[test]
    fn bengali_text_is_untouched() {
        assert_eq!(clean_html("<p>নির্বাচন ২০২৬</p>"), "<p>নির্বাচন ২০২৬</p>");
    }

    #[test]
    fn summary_text_has_spaces_and_no_captions() {
        let html = "<p>প্রথম।</p><figure><img src=\"/uploads/a.jpg\"><figcaption>ক্যাপশন</figcaption></figure><p>দ্বিতীয়।</p>";
        let text = plain_text(&without_figures(html));
        assert_eq!(text.split_whitespace().collect::<Vec<_>>().join(" "), "প্রথম। দ্বিতীয়।");
    }

    #[test]
    fn rejected_image_leaves_nothing_behind() {
        assert_eq!(clean_html("<p>a</p><img src=\"javascript:alert(1)\">"), "<p>a</p>");
    }

    #[test]
    fn plain_text_detects_empty() {
        assert!(plain_text("<p>&nbsp;</p><p><br></p>").trim().is_empty());
        assert_eq!(plain_text("<p>a &amp; b</p>").trim(), "a & b");
    }
}
