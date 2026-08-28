//! Task 2 Step 5: test the producer, not the formatter.
//!
//! `companion/crates/plugin/tests/fixtures/aqi_response.json` is a
//! captured-in-spirit, hand-authored-but-realistic payload for the
//! canonical `aqi` plugin (the plan's own manifest example uses `data.aqi`
//! / `data.category`). The plan's example source is `example.invalid`, so
//! nothing can actually be fetched from it -- the controller ruling on this
//! task is explicit that the temptation in that situation is to hand-build
//! a fixture already shaped like what the evaluator wants, which is exactly
//! the hole that hid three device defects for a whole stage (finding 3).
//!
//! This fixture is deliberately NOT that: it has an envelope (`status`,
//! `meta`) around the payload, four levels of object nesting
//! (`payload.current.pollutants.pm25.value`), arrays (`forecast`,
//! `attributions`, `station.geo`), JSON `null`s in three different
//! positions (a pollutant reading, an attribution URL, a whole forecast
//! day), a legacy field a real API would plausibly still serve as a string
//! even though it looks numeric (`current.legacy_index`), and a long list
//! of sibling fields no expression below ever reads (`station.geo`,
//! `station.url`, `current.dominant_pollutant`, `current.observed_at`,
//! `sync.*`, `meta.*`, `attributions[].name`, `forecast[].day`).
//!
//! `payload` (not the envelope root) is what `EvalContext` evaluates
//! against here, standing in for a provider layer that already checked
//! `status == "ok"` and handed the evaluator the inner value -- envelope
//! validation is a provider concern (a later task), not this module's.

use std::collections::HashMap;

use plugin::{EvalContext, EvalValue, Expr, FUEL_BUDGET, Fuel, Glyph, build_icon_map};

const FIXTURE: &str = include_str!("fixtures/aqi_response.json");

fn payload() -> serde_json::Value {
    let root: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture is valid JSON");
    assert_eq!(root["status"], "ok");
    root["payload"].clone()
}

fn icons() -> HashMap<String, u32> {
    // Stands in for a manifest's `[[assets]]` icon-font glyph table
    // (Task 1's `manifest::Glyph`), built the way `compile.rs` (Task 3)
    // will build it: from the parsed manifest, via `build_icon_map`.
    build_icon_map(&[
        Glyph {
            name: "moderate".to_string(),
            codepoint: 0xE003,
        },
        Glyph {
            name: "unhealthy-for-sensitive-groups".to_string(),
            codepoint: 0xE004,
        },
    ])
}

fn eval(source: &str, data: &serde_json::Value, icons: &HashMap<String, u32>) -> EvalValue {
    let mut fuel = Fuel::new(FUEL_BUDGET);
    Expr::parse(source)
        .unwrap_or_else(|err| panic!("{source:?} failed to parse: {err:?}"))
        .eval(&EvalContext::new(data, icons), &mut fuel)
        .unwrap_or_else(|err| panic!("{source:?} failed to eval: {err:?}"))
}

#[test]
fn a_top_level_numeric_field_reads_through() {
    let data = payload();
    assert_eq!(
        eval("data.current.aqi", &data, &icons()),
        EvalValue::Number(42.0)
    );
}

#[test]
fn deeply_nested_field_access_reaches_a_leaf_reading() {
    let data = payload();
    assert_eq!(
        eval("data.current.pollutants.pm25.value", &data, &icons()),
        EvalValue::Number(42.3)
    );
}

#[test]
fn a_null_leaf_in_real_nested_data_is_missing() {
    let data = payload();
    assert_eq!(
        eval("data.current.pollutants.o3.value", &data, &icons()),
        EvalValue::Missing
    );
}

#[test]
fn default_recovers_a_real_null_reading_with_a_fallback() {
    let data = payload();
    assert_eq!(
        eval(
            r#"default(data.current.pollutants.o3.value, "n/a")"#,
            &data,
            &icons()
        ),
        EvalValue::Text("n/a".to_string())
    );
}

#[test]
fn array_index_reaches_a_real_forecast_row() {
    let data = payload();
    assert_eq!(
        eval("data.forecast[0].aqi", &data, &icons()),
        EvalValue::Number(48.0)
    );
}

#[test]
fn a_null_array_element_field_is_missing_not_a_panic() {
    let data = payload();
    assert_eq!(
        eval("data.forecast[2].aqi", &data, &icons()),
        EvalValue::Missing
    );
    assert_eq!(
        eval("data.forecast[2].category", &data, &icons()),
        EvalValue::Missing
    );
}

#[test]
fn an_out_of_range_forecast_index_is_missing_not_a_panic() {
    let data = payload();
    assert_eq!(
        eval("data.forecast[99].aqi", &data, &icons()),
        EvalValue::Missing
    );
}

#[test]
fn upper_and_lower_operate_on_a_real_category_string() {
    let data = payload();
    assert_eq!(
        eval("upper(data.current.category)", &data, &icons()),
        EvalValue::Text("MODERATE".to_string())
    );
    assert_eq!(
        eval("lower(upper(data.current.category))", &data, &icons()),
        EvalValue::Text("moderate".to_string())
    );
}

#[test]
fn round_formats_a_real_fractional_reading() {
    let data = payload();
    assert_eq!(
        eval(
            "round(data.current.pollutants.pm25.value, 0)",
            &data,
            &icons()
        ),
        EvalValue::Number(42.0)
    );
    assert_eq!(
        eval(
            "round(data.current.pollutants.no2.value, 0)",
            &data,
            &icons()
        ),
        EvalValue::Number(5.0)
    );
}

#[test]
fn truncate_bounds_a_real_long_description_field() {
    let data = payload();
    assert_eq!(
        eval("truncate(data.station.description, 12)", &data, &icons()),
        EvalValue::Text("Rooftop sens".to_string())
    );
}

#[test]
fn icon_resolves_the_current_category_through_the_manifests_glyph_map() {
    let data = payload();
    let value = eval("icon(data.current.category)", &data, &icons());
    assert_eq!(
        value,
        EvalValue::Text(char::from_u32(0xE003).unwrap().to_string())
    );
}

#[test]
fn icon_on_a_category_the_icon_font_never_registered_is_missing() {
    // forecast[1].category is "unhealthy-for-sensitive-groups", which IS
    // registered; forecast[2].category is null -> Missing before icon()
    // even runs. Exercise a name genuinely absent from the map instead.
    let data = payload();
    assert_eq!(
        eval(r#"icon("hazardous")"#, &data, &icons()),
        EvalValue::Missing
    );
}

#[test]
fn a_numeric_looking_field_a_real_api_serves_as_a_string_stays_text() {
    // This is the finding-3 case in miniature: `legacy_index` LOOKS like a
    // number a naive author would `round()`, but this real-shaped API
    // serves it as a JSON string. The evaluator must not silently coerce
    // it -- it must come back as Text, and a function that requires a
    // Number must total out to Missing rather than guessing.
    let data = payload();
    assert_eq!(
        eval("data.current.legacy_index", &data, &icons()),
        EvalValue::Text("42".to_string())
    );
    assert_eq!(
        eval("round(data.current.legacy_index, 0)", &data, &icons()),
        EvalValue::Missing
    );
}

#[test]
fn a_station_id_that_looks_numeric_also_stays_text() {
    let data = payload();
    assert_eq!(
        eval("data.station.id", &data, &icons()),
        EvalValue::Text("8923".to_string())
    );
}

#[test]
fn a_boolean_leaf_reads_through_as_bool() {
    let data = payload();
    assert_eq!(
        eval("data.sync.degraded", &data, &icons()),
        EvalValue::Bool(false)
    );
}

#[test]
fn a_field_nothing_here_ever_reads_still_does_not_break_evaluation_of_others() {
    // `station.geo` is a two-element array of floats; no expression above
    // touches it. Reading it directly proves the evaluator tolerates a
    // sibling shape it was never asked about, rather than the fixture
    // secretly needing every field to be read for the file to be valid.
    let data = payload();
    assert_eq!(
        eval("data.station.geo[0]", &data, &icons()),
        EvalValue::Number(40.7128)
    );
    // And the compound value itself (no further indexing) is Missing, not
    // a panic -- an array has no scalar EvalValue representation.
    assert_eq!(
        eval("data.station.geo", &data, &icons()),
        EvalValue::Missing
    );
}

#[test]
fn a_key_absent_from_the_real_payload_is_missing_not_an_error() {
    let data = payload();
    assert_eq!(
        eval("data.current.pollutants.co2.value", &data, &icons()),
        EvalValue::Missing
    );
}
