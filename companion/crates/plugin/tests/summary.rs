//! Plugin-parity Task 1: the manifest-v2 `summary` expression, evaluated the
//! way `ServerProviderRefresher` will evaluate it at refresh time -- against
//! the snapshot's raw value, into one bounded headline or nothing.

use plugin::{ExprError, MAX_SUMMARY_LEN, SummaryError, evaluate_summary, parse_manifest};
use providers::ProviderSnapshot;

const AQI_FIXTURE: &str = include_str!("fixtures/aqi_response.json");

fn v2_manifest(summary_line: &str) -> String {
    format!(
        "manifest_version = 2\nname = \"aqi\"\nversion = \"1.0.0\"\n{summary_line}\n\n\
         [source]\nkind = \"json\"\nurl = \"https://example.invalid/aqi.json\"\nrefresh_minutes = 15\n\n\
         [template]\nkind = \"scene\"\n"
    )
}

fn snapshot(value: serde_json::Value) -> ProviderSnapshot<serde_json::Value> {
    ProviderSnapshot {
        value,
        refreshed_at: None,
        age: None,
        stale: false,
        error: None,
    }
}

fn aqi_payload() -> serde_json::Value {
    let root: serde_json::Value = serde_json::from_str(AQI_FIXTURE).expect("fixture is valid JSON");
    assert_eq!(root["status"], "ok");
    root["payload"].clone()
}

#[test]
fn an_undeclared_summary_evaluates_to_none() {
    let manifest = parse_manifest(&v2_manifest("")).expect("v2 without summary parses");
    assert_eq!(
        evaluate_summary(&manifest, &snapshot(aqi_payload())),
        Ok(None)
    );
}

#[test]
fn a_data_expression_evaluates_against_the_snapshot_value() {
    let manifest = parse_manifest(&v2_manifest("summary = \"{{ data.current.aqi }}\""))
        .expect("summary parses");
    assert_eq!(
        evaluate_summary(&manifest, &snapshot(aqi_payload())),
        Ok(Some("42".to_string()))
    );
}

#[test]
fn a_literal_summary_is_returned_as_written() {
    let manifest = parse_manifest(&v2_manifest("summary = \"Outside\"")).expect("literal parses");
    assert_eq!(
        evaluate_summary(&manifest, &snapshot(serde_json::json!({}))),
        Ok(Some("Outside".to_string()))
    );
}

#[test]
fn a_missing_value_evaluates_to_none_not_an_empty_string() {
    let manifest = parse_manifest(&v2_manifest("summary = \"{{ data.current.aqi }}\""))
        .expect("summary parses");
    assert_eq!(
        evaluate_summary(&manifest, &snapshot(serde_json::json!({}))),
        Ok(None)
    );
}

#[test]
fn a_stale_error_snapshot_still_evaluates_its_last_good_value() {
    let manifest = parse_manifest(&v2_manifest("summary = \"{{ data.current.aqi }}\""))
        .expect("summary parses");
    let mut stale = snapshot(aqi_payload());
    stale.stale = true;
    stale.error = Some("upstream request timed out".to_string());
    assert_eq!(
        evaluate_summary(&manifest, &stale),
        Ok(Some("42".to_string()))
    );
}

#[test]
fn output_one_byte_past_the_cap_is_truncated_on_a_char_boundary() {
    let manifest =
        parse_manifest(&v2_manifest("summary = \"{{ data.headline }}\"")).expect("summary parses");
    let eleven_euros = "€".repeat(11); // 33 bytes: the 32-byte cap falls mid-character
    let data = serde_json::json!({ "headline": eleven_euros });
    let summary = evaluate_summary(&manifest, &snapshot(data))
        .expect("evaluates")
        .expect("declared");
    assert_eq!(summary, "€".repeat(10));
    assert!(summary.len() <= MAX_SUMMARY_LEN);
}

#[test]
fn output_exactly_at_the_cap_is_kept_whole() {
    let exact = "a".repeat(MAX_SUMMARY_LEN);
    let manifest =
        parse_manifest(&v2_manifest(&format!("summary = \"{exact}\""))).expect("literal parses");
    assert_eq!(
        evaluate_summary(&manifest, &snapshot(serde_json::json!({}))),
        Ok(Some(exact))
    );
}

#[test]
fn a_partial_interpolation_is_a_named_error() {
    let manifest = parse_manifest(&v2_manifest("summary = \"AQI {{ data.current.aqi }}\""))
        .expect("the shape parses; interpolation is judged at evaluation");
    assert_eq!(
        evaluate_summary(&manifest, &snapshot(aqi_payload())),
        Err(SummaryError::MalformedPartialInterpolation {
            text: "AQI {{ data.current.aqi }}".to_string()
        })
    );
}

#[test]
fn an_unknown_identifier_is_a_named_expression_error() {
    let manifest = parse_manifest(&v2_manifest("summary = \"{{ bogus }}\"")).expect("shape parses");
    assert_eq!(
        evaluate_summary(&manifest, &snapshot(aqi_payload())),
        Err(SummaryError::Expression(ExprError::UnknownIdentifier {
            name: "bogus".to_string()
        }))
    );
}
