import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { describeCatalog } from "../src/main";
import type { FaceDefinition } from "../src/face";
import { SQUARE_SVG } from "./plugins/test_support";

// Pins `describeCatalog`'s exact output across the TypeScript/Rust boundary. The
// 2026-09-28 review found that `refreshSeconds` (camelCase, this side) was never
// reaching `crates/server/src/data_cards/faces_package.rs`'s `CatalogFace`, which
// declares `refresh_seconds` with no `#[serde(rename)]` and no
// `deny_unknown_fields` -- so the mismatched key deserialized to `None`, silently,
// on every run. Neither language's own unit tests could catch that alone: this
// side's tests never round-tripped through Rust, and the Rust side's own test used
// the correct snake_case key, which is exactly why it hid.
//
// `test/catalog-sample.json` is committed, real `describeCatalog` output for the
// two fixture faces below -- one that declares `refreshSeconds` and `tap`, one that
// declares neither. This test asserts the file on disk still matches what the
// function produces TODAY, so the sample cannot go stale relative to this side. A
// Rust test deserializes the same file; if either language's shape of this JSON
// drifts, one of the two tests fails.

const SAMPLE_FACES: FaceDefinition[] = [
  {
    kind: "sample-a",
    label: "Sample A",
    fields: [{ type: "text", key: "user", label: "Username", placeholder: "octocat" }],
    tap: "Tap for detail.",
    refreshSeconds: 300,
    render: async () => SQUARE_SVG,
  },
  {
    kind: "sample-b",
    label: "Sample B",
    fields: [],
    render: async () => SQUARE_SVG,
  },
];

test("catalog-sample.json is exactly describeCatalog's current output for the fixture faces", () => {
  const produced = describeCatalog(SAMPLE_FACES);
  const committed = readFileSync(`${import.meta.dir}/catalog-sample.json`, "utf8");
  expect(produced).toBe(committed);
});

test("a declared cadence is the snake_case key the Rust struct actually reads", () => {
  const catalog = JSON.parse(describeCatalog(SAMPLE_FACES)) as Record<string, unknown>[];
  const a = catalog[0] as { refresh_seconds?: number };
  expect(a.refresh_seconds).toBe(300);
  expect("refreshSeconds" in a).toBe(false);
});

test("no declared cadence is an absent key, never an explicit null", () => {
  const catalog = JSON.parse(describeCatalog(SAMPLE_FACES)) as Record<string, unknown>[];
  const b = catalog[1] as Record<string, unknown>;
  expect("refresh_seconds" in b).toBe(false);
  expect(JSON.stringify(b)).not.toContain("refresh_seconds");
});
