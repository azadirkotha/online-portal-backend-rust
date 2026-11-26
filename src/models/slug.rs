/// Turns a title into a URL slug.
///
/// Unlike an ASCII-only slugifier this KEEPS non-Latin letters (Bengali, Arabic, ...),
/// otherwise a name like "খেলাধুলা" would produce an empty slug. Only whitespace,
/// ASCII punctuation and a few common typographic marks act as separators.
pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_dash = true; // avoids a leading dash

    for c in input.trim().chars() {
        let separator = c.is_whitespace()
            || c.is_control()
            || c.is_ascii_punctuation()
            || matches!(c, '\u{0964}' | '\u{0965}' | '“' | '”' | '‘' | '’' | '–' | '—' | '…');

        if separator {
            if !last_was_dash {
                out.push('-');
                last_was_dash = true;
            }
        } else {
            out.extend(c.to_lowercase());
            last_was_dash = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn ascii() {
        assert_eq!(slugify("  World News & Politics! "), "world-news-politics");
    }

    #[test]
    fn bengali_is_preserved() {
        assert_eq!(slugify("খেলাধুলা ও বিনোদন"), "খেলাধুলা-ও-বিনোদন");
    }
}
