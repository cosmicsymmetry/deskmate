use serde::{Deserialize, Serialize};

use crate::ValidationIssue;

/// The shared body for the admin API's typed configuration-validation failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AdminConfigErrorBody {
    InvalidConfig { issues: Vec<ValidationIssue> },
}

/// The admin API's plugin registry, serialized by the server and deserialized
/// by the Mac app. One type, so the two cannot drift; the first five entry
/// fields keep the shipped key order and the rest are additive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCatalog {
    pub plugins: Vec<PluginCatalogEntry>,
    pub load_failures: Vec<PluginLoadFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCatalogEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub node_count: usize,
    pub assets: Vec<PluginCatalogAsset>,
    /// Manifest v2's `display_name`. `None` on a v1 manifest; the reader falls
    /// back to `id` rather than inventing a name here.
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub manifest_version: u8,
    pub template: PluginTemplateKind,
    pub refresh_minutes: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginCatalogAsset {
    pub file: String,
    pub kind: String,
    pub byte_length: usize,
    pub digest: String,
}

/// Deliberately NOT re-exported from the crate root: the server already owns a
/// `plugin_registry::PluginLoadFailure`, and two same-named types in one scope
/// is a trap. Reach this one as `app_core::admin::PluginLoadFailure`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLoadFailure {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginTemplateKind {
    DisplayList,
    Svg,
}

/// What a card's preview stage shows. A frame exists exactly for `Fresh` and
/// `Stale`; a message exists exactly for `Error` and `Waiting`. `Waiting` is
/// the ordinary pre-first-fetch state and must stay distinguishable from a
/// fault -- it prints its own state word, never the "No data yet" badge, which
/// means "a real frame rendered from sample data".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CardPreviewState {
    Fresh,
    Stale,
    Error,
    Waiting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardPreviewResponse {
    pub png_base64: Option<String>,
    pub state: CardPreviewState,
    pub message: Option<String>,
    pub refreshed_at_unix_ms: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ValidationCode;

    #[test]
    fn invalid_config_body_round_trips_between_server_and_clients() {
        let body = AdminConfigErrorBody::InvalidConfig {
            issues: vec![ValidationIssue {
                path: "cards[0].title".into(),
                code: ValidationCode::Empty,
                message: "Choose a title.".into(),
            }],
        };

        let json = serde_json::to_string(&body).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"invalid-config","issues":[{"path":"cards[0].title","code":"empty","message":"Choose a title."}]}"#
        );
        assert_eq!(
            serde_json::from_str::<AdminConfigErrorBody>(&json).unwrap(),
            body
        );
    }

    fn catalog_entry() -> PluginCatalogEntry {
        PluginCatalogEntry {
            id: "aqi".into(),
            name: "aqi".into(),
            version: "1.0.0".into(),
            node_count: 4,
            assets: vec![PluginCatalogAsset {
                file: "icons.ttf".into(),
                kind: "icon-font".into(),
                byte_length: 12,
                digest: "ab12".into(),
            }],
            display_name: Some("Air quality".into()),
            description: None,
            manifest_version: 2,
            template: PluginTemplateKind::DisplayList,
            refresh_minutes: 15,
        }
    }

    /// The first five keys, in this order, are byte-for-byte what the server's
    /// private `PluginResponse` already serializes (`server/src/admin.rs:95-101`).
    /// The five that follow are additive, so an old consumer reads the same
    /// bytes it read before. Pin the whole string: a reordering is a silent
    /// break for nothing.
    #[test]
    fn a_catalog_entry_keeps_the_shipped_keys_and_adds_the_new_ones_after_them() {
        assert_eq!(
            serde_json::to_string(&catalog_entry()).unwrap(),
            r#"{"id":"aqi","name":"aqi","version":"1.0.0","node_count":4,"assets":[{"file":"icons.ttf","kind":"icon-font","byte_length":12,"digest":"ab12"}],"display_name":"Air quality","description":null,"manifest_version":2,"template":"display-list","refresh_minutes":15}"#
        );
    }

    #[test]
    fn a_catalog_round_trips_between_the_server_and_the_mac() {
        let catalog = PluginCatalog {
            plugins: vec![catalog_entry()],
            load_failures: vec![PluginLoadFailure {
                id: "broken".into(),
                error: "manifest is not valid TOML".into(),
            }],
        };
        let json = serde_json::to_string(&catalog).unwrap();
        assert_eq!(
            json,
            format!(
                r#"{{"plugins":[{}],"load_failures":[{{"id":"broken","error":"manifest is not valid TOML"}}]}}"#,
                serde_json::to_string(&catalog_entry()).unwrap()
            )
        );
        assert_eq!(
            serde_json::from_str::<PluginCatalog>(&json).unwrap(),
            catalog
        );
    }

    #[test]
    fn an_svg_template_and_every_preview_state_serialize_as_the_wire_words() {
        assert_eq!(
            serde_json::to_string(&PluginTemplateKind::Svg).unwrap(),
            r#""svg""#
        );
        for (state, word) in [
            (CardPreviewState::Fresh, r#""fresh""#),
            (CardPreviewState::Stale, r#""stale""#),
            (CardPreviewState::Error, r#""error""#),
            (CardPreviewState::Waiting, r#""waiting""#),
        ] {
            assert_eq!(serde_json::to_string(&state).unwrap(), word);
        }
    }

    /// Spec 4.2's invariant, expressed as bytes: a frame exactly when the state
    /// is fresh or stale, a message exactly when it is error or waiting.
    #[test]
    fn a_waiting_preview_carries_a_message_and_no_frame() {
        let response = CardPreviewResponse {
            png_base64: None,
            state: CardPreviewState::Waiting,
            message: Some("Waiting for the first refresh".into()),
            refreshed_at_unix_ms: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(
            json,
            r#"{"png_base64":null,"state":"waiting","message":"Waiting for the first refresh","refreshed_at_unix_ms":null}"#
        );
        assert_eq!(
            serde_json::from_str::<CardPreviewResponse>(&json).unwrap(),
            response
        );
    }
}
