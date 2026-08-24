pub(super) fn truncate_smart(s: &str, max_len: usize) -> String {
    let char_len = s.chars().count();
    if char_len <= max_len {
        return s.to_string();
    }
    if max_len <= 3 {
        return "...".to_string();
    }

    let target = max_len - 3;
    let prefix = truncate_chars(s, target);

    if let Some(pos) = prefix.rfind(' ') {
        let before = &prefix[..pos];
        let pos_chars = before.chars().count();
        if pos_chars > target / 2 {
            return format!("{}...", before);
        }
    }
    format!("{}...", prefix)
}

pub(crate) fn truncate_chars(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

pub(super) fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let truncated = truncate_chars(s, max_chars.saturating_sub(1));
    format!("{}…", truncated)
}

/// Count how many lines `wrap_text` would produce for the given text and width.
pub(crate) fn wrap_line_count(text: &str, max_chars: usize) -> usize {
    if max_chars == 0 {
        return 1;
    }
    let mut count = 0;
    for paragraph in text.split('\n') {
        let mut current_len = 0usize;
        let mut has_content = false;
        for word in paragraph.split_whitespace() {
            let word_len = word.chars().count();
            if current_len == 0 {
                current_len = word_len;
                has_content = true;
            } else if current_len + 1 + word_len <= max_chars {
                current_len += 1 + word_len;
            } else {
                count += 1;
                current_len = word_len;
                has_content = true;
            }
        }
        if has_content || count == 0 {
            count += 1;
        }
    }
    count.max(1)
}
