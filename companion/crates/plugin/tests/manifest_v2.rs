//! Stage 4 Task 1 contract tests: v1 remains frozen while v2 is explicit.

use app_core::BakedFontMetrics;
use plugin::{
    Asset, CompileError, ManifestError, Node, PluginManifest, Repeat, Source, compile_scene,
    parse_manifest,
};
use protocol::SceneNode;
use providers::ProviderSnapshot;
use serde::Deserialize;

const V1_HEADER: &str = r#"
name = "compat"
version = "1.0.0"

[source]
kind = "json"
url = "https://example.invalid/data.json"
refresh_minutes = 15
"#;

const V2_HEADER: &str = include_str!("fixtures/manifest_v2_scene.toml");

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FrozenV1Manifest {
    name: String,
    version: String,
    source: FrozenV1Source,
    #[serde(default)]
    assets: Vec<Asset>,
    #[serde(default)]
    nodes: Vec<Node>,
    #[serde(default)]
    repeats: Vec<Repeat>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum FrozenV1Source {
    Json { url: String, refresh_minutes: u32 },
}

fn assert_frozen_v1_fields_are_identical(source: &str) {
    let frozen: FrozenV1Manifest = toml::from_str(source).expect("frozen v1 shape parses");
    let current = parse_manifest(source).expect("current parser accepts frozen v1");

    assert!(current.is_manifest_v1());
    assert!(current.is_scene_template());
    assert_eq!(current.name, frozen.name);
    assert_eq!(current.version, frozen.version);
    match (&current.source, frozen.source) {
        (
            Source::Json {
                url,
                refresh_minutes,
                ..
            },
            FrozenV1Source::Json {
                url: frozen_url,
                refresh_minutes: frozen_refresh_minutes,
            },
        ) => {
            assert_eq!(url, &frozen_url);
            assert_eq!(*refresh_minutes, frozen_refresh_minutes);
            let Source::Json { root, .. } = &current.source;
            assert_eq!(root, &None, "v1 must synthesize no source root");
        }
    }
    assert_eq!(current.assets, frozen.assets);
    assert_eq!(current.nodes, frozen.nodes);
    assert_eq!(current.repeats, frozen.repeats);
}

#[test]
fn the_frozen_v1_fixture_manifests_reparse_with_every_frozen_field_unchanged() {
    // Byte-exact copies of `companion/plugins/{aqi,agenda}/manifest.toml` as
    // they shipped under v1, frozen when the curated plugins moved to v2
    // (plugin-parity Task 1) so the v1 contract keeps real coverage.
    assert_frozen_v1_fields_are_identical(include_str!("fixtures/manifest_v1_aqi.toml"));
    assert_frozen_v1_fields_are_identical(include_str!("fixtures/manifest_v1_agenda.toml"));
}

#[test]
fn v1_rejects_every_v2_only_contract_field() {
    let cases = [
        (
            "manifest discriminator with a v1 value",
            V1_HEADER.replacen("name =", "manifest_version = 1\nname =", 1),
        ),
        (
            "source root",
            V1_HEADER.replacen(
                "refresh_minutes = 15",
                "refresh_minutes = 15\nroot = \"payload\"",
                1,
            ),
        ),
        (
            "template table",
            format!("{V1_HEADER}\n[template]\nkind = \"scene\"\n"),
        ),
        (
            "arc end binding",
            format!(
                "{V1_HEADER}\n[[nodes]]\nkind = \"arc\"\ncx = 100\ncy = 100\nr = 50\nstart_deg = 0\nend_deg = 360\nend_binding = \"{{{{ timer.pct }}}}\"\nwidth = 8\ncolor = 0xffffff\n"
            ),
        ),
        (
            "bound line geometry",
            format!(
                "{V1_HEADER}\n[[nodes]]\nkind = \"line\"\npivot_x = 100\npivot_y = 100\nlength = 50\nangle_binding = \"{{{{ time:angle:minute }}}}\"\nwidth = 4\ncolor = 0xffffff\n"
            ),
        ),
    ];

    for (name, source) in cases {
        assert!(
            parse_manifest(&source).is_err(),
            "v1 unexpectedly accepted {name}"
        );
    }
}

#[test]
fn contract_discriminator_accepts_only_absent_or_exactly_two() {
    let v1_value = V1_HEADER.replacen("name =", "manifest_version = 1\nname =", 1);
    let future_value = V1_HEADER.replacen("name =", "manifest_version = 3\nname =", 1);
    assert_eq!(
        parse_manifest(&v1_value).expect_err("manifest_version = 1 is not v1"),
        ManifestError::UnsupportedManifestVersion { value: 1 }
    );
    assert_eq!(
        parse_manifest(&future_value).expect_err("unknown future version must fail closed"),
        ManifestError::UnsupportedManifestVersion { value: 3 }
    );
}

#[test]
fn explicit_v2_scene_root_arc_and_bound_line_forms_parse() {
    let source = format!(
        "{V2_HEADER}\n\
         [[nodes]]\nkind = \"arc\"\ncx = 100\ncy = 100\nr = 50\nstart_deg = 0\nend_deg = 360\nend_binding = \"{{{{ timer.pct }}}}\"\nwidth = 8\ncolor = 0xffffff\n\n\
         [[nodes]]\nkind = \"line\"\npivot_x = 100\npivot_y = 100\nlength = 50\nangle_binding = \"{{{{ time:angle:minute }}}}\"\nwidth = 4\ncolor = 0xffffff\n"
    );

    let manifest = parse_manifest(&source).expect("explicit v2 scene form parses");
    assert!(manifest.is_manifest_v2());
    assert!(manifest.is_scene_template());
    assert_eq!(manifest.nodes.len(), 2);
}

#[test]
fn explicit_v2_may_omit_root_without_inventing_an_envelope_convention() {
    let source = V2_HEADER.replace("root = \"payload\"\n", "");
    let manifest = parse_manifest(&source).expect("v2 root is optional");
    let Source::Json { root, .. } = manifest.source;
    assert_eq!(root, None);
}

#[test]
fn explicit_v2_svg_form_parses() {
    let source = include_str!("fixtures/manifest_v2_svg.toml");

    let manifest = parse_manifest(source).expect("explicit v2 SVG form parses");
    assert!(manifest.is_manifest_v2());
    assert_eq!(manifest.svg_template_file(), Some("face.svg"));
    assert!(manifest.nodes.is_empty());
    assert!(manifest.repeats.is_empty());
}

#[test]
fn svg_template_is_not_silently_compiled_as_a_blank_native_scene() {
    let manifest =
        parse_manifest(include_str!("fixtures/manifest_v2_svg.toml")).expect("SVG fixture parses");
    assert_eq!(
        compile_scene(&manifest, &empty_snapshot(), &BakedFontMetrics::SHIPPED, 1,)
            .expect_err("SVG must stay on the registry/raster plumbing path"),
        CompileError::TemplateNotScene
    );
}

#[test]
fn explicit_v2_requires_a_template_and_svg_requires_its_file() {
    let missing_template = V2_HEADER.replace("\n[template]\nkind = \"scene\"\n", "\n");
    assert_eq!(
        parse_manifest(&missing_template).expect_err("v2 template is required"),
        ManifestError::MissingV2Template
    );

    let missing_file = V2_HEADER.replacen("kind = \"scene\"", "kind = \"svg\"", 1);
    assert!(matches!(
        parse_manifest(&missing_file),
        Err(ManifestError::Toml(_))
    ));
}

fn empty_snapshot() -> ProviderSnapshot<serde_json::Value> {
    ProviderSnapshot {
        value: serde_json::json!({}),
        refreshed_at: None,
        age: None,
        stale: false,
        error: None,
    }
}

#[test]
fn v2_arc_and_bound_line_bindings_reach_the_protocol_scene() {
    let source = format!(
        "{V2_HEADER}\n\
         [[nodes]]\nkind = \"arc\"\ncx = 100\ncy = 100\nr = 50\nstart_deg = 0\nend_deg = 360\nend_binding = \"{{{{ timer.permille }}}}\"\nwidth = 8\ncolor = 0xffffff\n\n\
         [[nodes]]\nkind = \"line\"\npivot_x = 100\npivot_y = 100\nlength = 50\nangle_binding = \"{{{{ time:angle:hour }}}}\"\nwidth = 4\ncolor = 0xffffff\n"
    );
    let manifest = parse_manifest(&source).expect("v2 binding manifest parses");
    let scene = compile_scene(&manifest, &empty_snapshot(), &BakedFontMetrics::SHIPPED, 7)
        .expect("v2 bindings compile");

    let SceneNode::Arc(arc) = &scene.nodes[0] else {
        panic!("node 0 is not an arc: {:?}", scene.nodes[0]);
    };
    assert_eq!(arc.end_binding, "timer.permille");
    let SceneNode::Line(line) = &scene.nodes[1] else {
        panic!("node 1 is not a line: {:?}", scene.nodes[1]);
    };
    assert!(line.xs.is_empty());
    assert!(line.ys.is_empty());
    assert_eq!((line.pivot_x, line.pivot_y, line.length), (100, 100, 50));
    assert_eq!(line.angle_binding, "time:angle:hour");
    protocol::validate_scene(&scene).expect("protocol remains final authority");
}

#[test]
fn every_protocol_token_allowed_in_each_new_position_compiles() {
    for binding in ["timer.pct", "timer.permille"] {
        let source = format!(
            "{V2_HEADER}\n[[nodes]]\nkind=\"arc\"\ncx=100\ncy=100\nr=50\nstart_deg=0\nend_deg=360\nend_binding=\"{{{{ {binding} }}}}\"\nwidth=4\ncolor=0\n"
        );
        let manifest = parse_manifest(&source).expect("arc binding shape parses");
        let scene = compile_scene(&manifest, &empty_snapshot(), &BakedFontMetrics::SHIPPED, 1)
            .expect("allowed arc binding compiles");
        let SceneNode::Arc(arc) = &scene.nodes[0] else {
            panic!("expected arc");
        };
        assert_eq!(arc.end_binding, binding);
    }

    for binding in ["time:angle:hour", "time:angle:minute"] {
        let source = format!(
            "{V2_HEADER}\n[[nodes]]\nkind=\"line\"\npivot_x=100\npivot_y=100\nlength=50\nangle_binding=\"{{{{ {binding} }}}}\"\nwidth=4\ncolor=0\n"
        );
        let manifest = parse_manifest(&source).expect("line binding shape parses");
        let scene = compile_scene(&manifest, &empty_snapshot(), &BakedFontMetrics::SHIPPED, 1)
            .expect("allowed line binding compiles");
        let SceneNode::Line(line) = &scene.nodes[0] else {
            panic!("expected line");
        };
        assert_eq!(line.angle_binding, binding);
    }
}

#[test]
fn v2_line_rejects_both_geometry_forms_and_neither_geometry_form() {
    let both = format!(
        "{V2_HEADER}\n[[nodes]]\nkind = \"line\"\npoints = [{{x=1,y=1}},{{x=2,y=2}}]\npivot_x = 100\npivot_y = 100\nlength = 50\nangle_binding = \"{{{{ time:angle:minute }}}}\"\nwidth = 4\ncolor = 0xffffff\n"
    );
    let neither = format!("{V2_HEADER}\n[[nodes]]\nkind = \"line\"\nwidth = 4\ncolor = 0xffffff\n");

    assert_eq!(
        parse_manifest(&both).expect_err("both geometries must fail"),
        ManifestError::AmbiguousLineGeometry
    );
    assert_eq!(
        parse_manifest(&neither).expect_err("missing geometry must fail"),
        ManifestError::MissingLineGeometry
    );
}

#[test]
fn v2_bound_line_rejects_an_incomplete_geometry_by_name() {
    let incomplete = format!(
        "{V2_HEADER}\n[[nodes]]\nkind = \"line\"\npivot_x = 100\npivot_y = 100\nangle_binding = \"{{{{ time:angle:minute }}}}\"\nwidth = 4\ncolor = 0xffffff\n"
    );
    assert_eq!(
        parse_manifest(&incomplete).expect_err("bound line length is required"),
        ManifestError::IncompleteBoundLineGeometry
    );
}

#[test]
fn live_looking_arc_binding_typo_is_never_compiled_as_a_literal() {
    let source = format!(
        "{V2_HEADER}\n[[nodes]]\nkind = \"arc\"\ncx = 100\ncy = 100\nr = 50\nstart_deg = 0\nend_deg = 360\nend_binding = \"{{{{ timer.velocity }}}}\"\nwidth = 8\ncolor = 0xffffff\n"
    );
    let manifest = parse_manifest(&source).expect("shape parses before binding validation");
    assert_eq!(
        compile_scene(&manifest, &empty_snapshot(), &BakedFontMetrics::SHIPPED, 1,)
            .expect_err("timer.velocity must be a named compile error"),
        CompileError::UnknownBinding {
            text: "timer.velocity".to_string()
        }
    );
}

#[test]
fn live_looking_line_binding_typo_is_never_compiled_as_a_literal() {
    let source = format!(
        "{V2_HEADER}\n[[nodes]]\nkind=\"line\"\npivot_x=100\npivot_y=100\nlength=50\nangle_binding=\"{{{{ timer.velocity }}}}\"\nwidth=4\ncolor=0\n"
    );
    let manifest = parse_manifest(&source).expect("shape parses before binding validation");
    assert_eq!(
        compile_scene(&manifest, &empty_snapshot(), &BakedFontMetrics::SHIPPED, 1,)
            .expect_err("timer.velocity must be a named compile error"),
        CompileError::UnknownBinding {
            text: "timer.velocity".to_string()
        }
    );
}

fn v2_source_with_root(root: &str) -> String {
    V2_HEADER.replacen("root = \"payload\"", &format!("root = \"{root}\""), 1)
}

#[test]
fn source_root_rejects_an_empty_dotted_segment() {
    assert_eq!(
        parse_manifest(&v2_source_with_root("payload..current"))
            .expect_err("empty root segment must fail"),
        ManifestError::InvalidSourceRoot {
            root: "payload..current".to_string()
        }
    );
}

#[test]
fn source_root_one_byte_past_its_length_cap_is_rejected_by_name() {
    let root = "a".repeat(PluginManifest::MAX_SOURCE_ROOT_LEN + 1);
    let error = parse_manifest(&v2_source_with_root(&root)).expect_err("root must be bounded");
    assert_eq!(
        error,
        ManifestError::StringTooLong {
            field: "source.root",
            limit: PluginManifest::MAX_SOURCE_ROOT_LEN,
            actual: PluginManifest::MAX_SOURCE_ROOT_LEN + 1,
        }
    );
}

#[test]
fn source_root_one_segment_past_the_expression_path_cap_is_rejected_by_name() {
    let root = std::iter::repeat_n("a", plugin::MAX_PATH_SEGMENTS + 1)
        .collect::<Vec<_>>()
        .join(".");
    let error = parse_manifest(&v2_source_with_root(&root)).expect_err("root depth is bounded");
    assert_eq!(
        error,
        ManifestError::TooManySourceRootSegments {
            limit: plugin::MAX_PATH_SEGMENTS,
            actual: plugin::MAX_PATH_SEGMENTS + 1,
        }
    );
}

#[test]
fn template_file_uses_the_same_single_normal_component_posture_as_assets() {
    for file in [
        "/etc/passwd",
        "../face.svg",
        "nested/face.svg",
        "./face.svg",
    ] {
        let source = format!(
            "manifest_version = 2\nname = \"svg\"\nversion = \"1.0.0\"\n\
             [source]\nkind = \"json\"\nurl = \"https://example.invalid/x\"\nrefresh_minutes = 15\n\
             [template]\nkind = \"svg\"\nfile = \"{file}\"\n"
        );
        assert_eq!(
            parse_manifest(&source).expect_err("hostile template path must fail"),
            ManifestError::InvalidTemplatePath {
                file: file.to_string()
            }
        );
    }
}

#[test]
fn template_file_one_byte_past_the_shared_filename_cap_is_rejected() {
    let file = "f".repeat(PluginManifest::MAX_FILE_NAME_LEN - ".svg".len() + 1) + ".svg";
    assert_eq!(file.len(), PluginManifest::MAX_FILE_NAME_LEN + 1);
    let source = format!(
        "manifest_version=2\nname=\"svg\"\nversion=\"1.0.0\"\n\
         [source]\nkind=\"json\"\nurl=\"https://example.invalid/x\"\nrefresh_minutes=15\n\
         [template]\nkind=\"svg\"\nfile=\"{file}\"\n"
    );
    assert_eq!(
        parse_manifest(&source).expect_err("template filename must be bounded"),
        ManifestError::StringTooLong {
            field: "template.file",
            limit: PluginManifest::MAX_FILE_NAME_LEN,
            actual: PluginManifest::MAX_FILE_NAME_LEN + 1,
        }
    );
}

#[test]
fn template_union_rejects_scene_files_and_svg_display_lists() {
    let scene_with_file = V2_HEADER.replacen(
        "kind = \"scene\"",
        "kind = \"scene\"\nfile = \"face.svg\"",
        1,
    );
    assert!(matches!(
        parse_manifest(&scene_with_file),
        Err(ManifestError::Toml(_))
    ));

    let svg_with_nodes =
        V2_HEADER.replacen("kind = \"scene\"", "kind = \"svg\"\nfile = \"face.svg\"", 1)
            + "\n[[nodes]]\nkind = \"rect\"\nx=0\ny=0\nw=1\nh=1\nradius=0\nfill=0\n";
    assert_eq!(
        parse_manifest(&svg_with_nodes).expect_err("SVG must forbid nodes"),
        ManifestError::SvgTemplateHasSceneContent
    );

    let svg_with_repeats =
        V2_HEADER.replacen("kind = \"scene\"", "kind = \"svg\"\nfile = \"face.svg\"", 1)
            + "\n[[repeats]]\nsource=\"rows\"\ndx=0\ndy=1\nnodes=[]\n";
    assert_eq!(
        parse_manifest(&svg_with_repeats).expect_err("SVG must forbid repeats"),
        ManifestError::SvgTemplateHasSceneContent
    );
}

#[test]
fn bindings_valid_elsewhere_are_rejected_in_the_wrong_numeric_or_angle_position() {
    let arc = format!(
        "{V2_HEADER}\n[[nodes]]\nkind = \"arc\"\ncx=100\ncy=100\nr=50\nstart_deg=0\nend_deg=360\nend_binding=\"{{{{ timer.status }}}}\"\nwidth=4\ncolor=0\n"
    );
    let line = format!(
        "{V2_HEADER}\n[[nodes]]\nkind = \"line\"\npivot_x=100\npivot_y=100\nlength=50\nangle_binding=\"{{{{ timer.pct }}}}\"\nwidth=4\ncolor=0\n"
    );
    for (field, source) in [("arc.end_binding", arc), ("line.angle_binding", line)] {
        let manifest = parse_manifest(&source).expect("shape parses");
        assert!(matches!(
            compile_scene(
                &manifest,
                &empty_snapshot(),
                &BakedFontMetrics::SHIPPED,
                1
            ),
            Err(CompileError::InvalidBindingPosition { field: actual, .. }) if actual == field
        ));
    }
}
