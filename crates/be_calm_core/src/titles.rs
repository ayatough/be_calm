//! Window-title blocklist. Allowing a browser allows every website, so this
//! is the cheap, permission-free way to keep video sites out: if a window
//! that is otherwise allowed shows one of these words in its title, the app
//! closes that tab (Ctrl+W) instead of the whole window.

/// Default keywords. Matched case-insensitively as substrings of the title.
pub const DEFAULT_BLOCKED_TITLES: &[&str] = &[
    "YouTube",
    "ニコニコ",
    "Twitch",
    "TikTok",
    "Netflix",
    "Prime Video",
    "ABEMA",
    "Disney+",
    "Hulu",
    "U-NEXT",
    "Bilibili",
    "Reddit",
    "Instagram",
    "Facebook",
    "Threads",
    "Pixiv",
];

/// Which keyword (if any) matches the title.
pub fn blocked_keyword<'a>(title: &str, keywords: &'a [String]) -> Option<&'a str> {
    let lower = title.to_lowercase();
    keywords
        .iter()
        .map(|k| k.trim())
        .filter(|k| !k.is_empty())
        .find(|k| lower.contains(&k.to_lowercase()))
}

/// Parse a newline- or comma-separated user list into keywords.
pub fn parse_keywords(text: &str) -> Vec<String> {
    text.split(['\n', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kws() -> Vec<String> {
        DEFAULT_BLOCKED_TITLES
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn matches_case_insensitive_substrings() {
        let k = kws();
        assert_eq!(
            blocked_keyword("(3) Lo-fi beats - YouTube - Google Chrome", &k),
            Some("YouTube")
        );
        assert_eq!(
            blocked_keyword("youtube.com/watch?v=x — Mozilla Firefox", &k),
            Some("YouTube")
        );
        assert_eq!(
            blocked_keyword("ニコニコ動画 - Microsoft Edge", &k),
            Some("ニコニコ")
        );
        assert_eq!(
            blocked_keyword("arXiv:2401.00001 - Google Chrome", &k),
            None
        );
    }

    #[test]
    fn empty_keywords_never_match() {
        assert_eq!(
            blocked_keyword("YouTube", &[String::new(), "  ".into()]),
            None
        );
        assert_eq!(blocked_keyword("YouTube", &[]), None);
    }

    #[test]
    fn parse_accepts_commas_and_newlines() {
        assert_eq!(
            parse_keywords("YouTube, Twitch\n\n  ニコニコ  ,"),
            vec!["YouTube", "Twitch", "ニコニコ"]
        );
    }
}
