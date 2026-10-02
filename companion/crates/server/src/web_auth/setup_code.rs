use std::sync::Mutex;

use rand::Rng as _;

const ALPHABET: &[u8] = b"23456789ABCDEFGHJKMNPQRSTUVWXYZ";
const CODE_LEN: usize = 8;

pub(crate) struct SetupCode {
    value: Mutex<Option<String>>,
}

impl SetupCode {
    pub(crate) fn generate() -> (Self, String) {
        let code = Self::inactive();
        let display = code.regenerate();
        (code, display)
    }

    pub(crate) fn regenerate(&self) -> String {
        let mut rng = rand::rng();
        let raw = (0..CODE_LEN)
            .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
            .collect::<String>();
        let display = format!("{}-{}", &raw[..4], &raw[4..]);
        *self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(raw);
        display
    }

    pub(crate) fn inactive() -> Self {
        Self {
            value: Mutex::new(None),
        }
    }

    pub(crate) fn matches(&self, presented: &str) -> bool {
        let normalized = normalize(presented);
        let value = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        value.as_deref().is_some_and(|expected| {
            crate::registry::constant_time_eq(expected.as_bytes(), normalized.as_bytes())
        })
    }

    pub(crate) fn erase(&self) {
        self.value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }

    pub(crate) fn is_active(&self) -> bool {
        self.value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some()
    }

    pub(crate) fn display_for_tests(&self) -> Option<String> {
        self.value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_deref()
            .map(|raw| format!("{}-{}", &raw[..4], &raw[4..]))
    }
}

fn normalize(presented: &str) -> String {
    presented
        .chars()
        .filter(|character| *character != '-' && !character.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

pub(crate) fn announce_setup_code(url: &url::Url, code: &str) {
    tracing::warn!(target: "deskmate_server::setup",
        "\n==============================================\n  Deskmate setup code: {code}\n  Open {url} to set up this server.\n=============================================="
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inactive_code_can_be_regenerated() {
        let code = SetupCode::inactive();
        assert!(!code.is_active());
        let display = code.regenerate();
        assert!(code.is_active());
        assert!(code.matches(&display));
        assert_eq!(display.len(), 9);
        assert_eq!(&display[4..5], "-");
        assert!(
            display
                .bytes()
                .enumerate()
                .all(|(i, b)| i == 4 || ALPHABET.contains(&b))
        );
    }

    #[test]
    fn generated_code_has_the_unambiguous_display_shape_and_is_erasable() {
        let (code, display) = SetupCode::generate();
        let bytes = display.as_bytes();
        assert_eq!(bytes.len(), 9);
        assert_eq!(bytes[4], b'-');
        assert!(
            bytes
                .iter()
                .enumerate()
                .all(|(index, byte)| index == 4 || ALPHABET.contains(byte))
        );
        assert!(code.matches(&display.to_lowercase()));
        assert!(code.matches(&display.replace('-', "")));
        assert!(code.is_active());
        code.erase();
        assert!(!code.is_active());
        assert!(!code.matches(&display));
    }
}
