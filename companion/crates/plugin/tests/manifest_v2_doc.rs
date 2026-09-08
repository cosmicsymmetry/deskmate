//! Task 6: the frozen manifest-v2 contract document, this crate's bounds, and the
//! manifests on disk cannot drift apart.
//!
//! `docs/plugins/manifest-v2.md` is a frozen contract other people read before they
//! write a manifest, and Task 1 owns every word of it. This test owns none of that
//! prose and changes none of it: it pins only the numbers and paths that also live
//! in code. A bound stated once in prose and once in a `const` will eventually
//! disagree, and the disagreement is invisible until someone's manifest is rejected
//! for a reason the document says is legal.

use std::path::{Path, PathBuf};

/// The curated plugins, all four of which Task 1 moved to `manifest_version = 2`.
const CURATED_IDS: [&str; 4] = ["agenda", "aqi", "claude-limits", "svg-aqi"];

/// The byte-exact v1 copies Task 1 froze before the curated manifests moved to v2.
/// They are all the coverage manifest v1 has left, and `manifest-v2.md`'s
/// `## V1 compatibility` section names both paths. If either the prose or the file
/// goes away, v1 coverage has quietly gone with it.
const V1_FIXTURES: [&str; 2] = [
    "companion/crates/plugin/tests/fixtures/manifest_v1_aqi.toml",
    "companion/crates/plugin/tests/fixtures/manifest_v1_agenda.toml",
];

fn repo_root() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is `companion/crates/plugin`.
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn contract_doc() -> String {
    read(&repo_root().join("docs/plugins/manifest-v2.md"))
}

/// True when `line` states `value` as a number in its own right, so that a row
/// claiming `MAX_SUMMARY_LEN` is 32 is not satisfied by the "32" hiding inside
/// some neighbouring figure.
fn states_number(line: &str, value: usize) -> bool {
    let text = value.to_string();
    line.match_indices(&text).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + text.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

#[test]
fn the_contract_document_states_every_display_bound_with_its_real_value() {
    let doc = contract_doc();
    for (name, value) in [
        ("MAX_DISPLAY_NAME_LEN", plugin::MAX_DISPLAY_NAME_LEN),
        ("MAX_DESCRIPTION_LEN", plugin::MAX_DESCRIPTION_LEN),
        ("MAX_SUMMARY_LEN", plugin::MAX_SUMMARY_LEN),
    ] {
        let mentions: Vec<&str> = doc.lines().filter(|line| line.contains(name)).collect();
        assert!(
            !mentions.is_empty(),
            "docs/plugins/manifest-v2.md never names `{name}`"
        );
        assert!(
            mentions.iter().any(|line| states_number(line, value)),
            "docs/plugins/manifest-v2.md names `{name}` but never states its value \
             {value}; the lines mentioning it are {mentions:?}"
        );
    }
}

#[test]
fn the_contract_document_names_both_presentation_key_errors() {
    let doc = contract_doc();
    for error in ["SummaryUsesDeviceBinding", "EmptyString"] {
        assert!(
            doc.contains(error),
            "docs/plugins/manifest-v2.md never names `ManifestError::{error}`, so an \
             author cannot look up why their manifest was rejected"
        );
    }
}

#[test]
fn manifest_v2_doc_records_the_three_presentation_keys_the_curated_plugins_use() {
    let doc = contract_doc();
    for id in CURATED_IDS {
        let source = read(
            &repo_root()
                .join("companion/plugins")
                .join(id)
                .join("manifest.toml"),
        );
        let manifest = plugin::parse_manifest(&source)
            .unwrap_or_else(|error| panic!("{id}/manifest.toml parses: {error:?}"));
        assert!(
            !manifest.is_manifest_v1(),
            "{id} is not manifest_version = 2"
        );
        assert!(
            manifest.display_name.is_some(),
            "{id} declares no display_name"
        );
        assert!(
            manifest.description.is_some(),
            "{id} declares no description"
        );
        assert!(manifest.summary.is_some(), "{id} declares no summary");
        assert!(
            doc.contains(&format!("`{id}`")),
            "manifest-v2.md never names `{id}`, so its move to v2 is unrecorded"
        );
    }
}

#[test]
fn the_v1_fixture_paths_the_contract_names_exist_and_still_parse_as_v1() {
    let doc = contract_doc();
    for path in V1_FIXTURES {
        assert!(
            doc.contains(path),
            "manifest-v2.md no longer names `{path}`; v1's only remaining coverage is \
             then unfindable from the contract that points at it"
        );
        let manifest = plugin::parse_manifest(&read(&repo_root().join(path)))
            .unwrap_or_else(|error| panic!("{path} parses: {error:?}"));
        assert!(manifest.is_manifest_v1(), "{path} is no longer manifest v1");
        assert_eq!(manifest.display_name, None, "{path} gained a v2 key");
        assert_eq!(manifest.description, None, "{path} gained a v2 key");
        assert_eq!(manifest.summary, None, "{path} gained a v2 key");
    }
}

#[test]
fn the_contract_no_longer_claims_the_shipped_manifests_are_v1() {
    let doc = contract_doc();
    for stale in ["byte-for-byte unchanged", "The two shipped v1 manifests"] {
        assert!(
            !doc.contains(stale),
            "manifest-v2.md still says {stale:?}; all four shipped manifests are v2 now"
        );
    }
}
