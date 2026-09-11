use serde::{Deserialize, Serialize};

use crate::ValidationIssue;

/// The shared body for the admin API's typed configuration-validation failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AdminConfigErrorBody {
    InvalidConfig { issues: Vec<ValidationIssue> },
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
}
