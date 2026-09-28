pub(crate) fn normalize_email(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    if email.is_empty() || email.len() > 254 {
        return None;
    }
    if email
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return None;
    }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return None;
    }
    Some(email)
}

#[cfg(test)]
mod tests {
    use super::normalize_email;

    #[test]
    fn normalises_and_rejects() {
        assert_eq!(
            normalize_email("  Rodion@Example.COM ").as_deref(),
            Some("rodion@example.com")
        );
        for bad in ["", "no-at", "a@@b.c", "a b@c.d", "@c.d", "a@", "a@b\0.c"] {
            assert_eq!(normalize_email(bad), None, "{bad:?}");
        }
        assert_eq!(
            normalize_email(&format!("{}@b.co", "a".repeat(260))),
            None,
            "over 254 bytes"
        );
    }
}
