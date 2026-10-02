# Plugin renderer resource audit

Audited 2026-10-02 for the runtime pinned in `companion/faces/bun.lock`.
The boundary is `plugins/card.ts` / `plugins/resources.ts`, shared by live server
renders and author previews. Invalid resource references raise a configuration
error before an engine can load them; the last accepted frame is retained.

## Satori 0.33.5

Inspected the installed package's `dist/index.js.map` source contents, including
all `fetch`, `resolveImageData` and `loadAdditionalAsset` call sites.

| Loading path | Boundary and regression coverage |
|---|---|
| `handler/preprocess.ts`, `handler/compute.ts`: `img.src`, SVG `image.href` / `xlinkHref` | Layout nodes become only host-constructed div/img elements. Only bounded data images reach img.src; extra SVG-node properties are discarded. |
| `builder/background-image.ts`: background URLs, also reached by background shorthand | Every style string is checked before Satori, including all layers and nested children. Tests assert zero fetch calls and a configuration refusal. |
| `builder/mask-image.ts` / `parser/mask.ts`: `maskImage`, `WebkitMaskImage` | Same resource validation and image budgets as backgrounds and img. Tests cover external and oversized data URLs and valid embedded images. |
| `satori.ts`: `loadAdditionalAsset`, `graphemeImages`, supplied fonts | Only the host supplies options, with bundled font bytes and no asset loader/grapheme URLs. Plugin properties cannot set these options. Font URLs and CSS at-rules in card output are refused. |
| `handler/image.ts`: data-URI SVGs and `handler/url-safety.ts`: server fetch | Embedded SVG is decoded and recursively checked before Satori sees it. URL validation does not rely on Satori's DNS/SSRF checks, cache, or its catch-and-log behavior on failed fetches. |
| Yoga/HarfBuzz WASM initialization | Engine-owned embedded/package assets; no plugin input selects their location. |

CSS escapes, comments, at-rules and indirect image functions are outside the card
subset. This avoids accepting a value that the engines interpret differently.
The URL scanner checks all style properties, not a list of known image properties.

## resvg-js 2.6.2

The npm binding uses the resvg fork pinned at `3495d870`, **not** the newer usvg
crate cached by the Rust workspace. Its [manifest](https://github.com/yisibl/resvg-js/blob/v2.6.2/Cargo.toml)
and [binding options](https://github.com/yisibl/resvg-js/blob/v2.6.2/src/options.rs)
establish the dependency and resolver behavior.

| Loading path | Boundary and regression coverage |
|---|---|
| [Image resolver](https://github.com/zimond/resvg/blob/3495d870/crates/usvg-parser/src/image.rs): image href, file paths, embedded SVG and text/plain sniffing | XML is parsed before resvg, including arbitrary namespace prefixes and decoded numeric entities. Image hrefs require bounded data images; text/plain, image/raw and compressed SVG are refused. Nested SVGs are checked recursively. |
| [Filter images](https://github.com/zimond/resvg/blob/3495d870/crates/usvg-parser/src/filter.rs): `feImage` | Data-only image references, including fragments that could otherwise fall back to a local filename. |
| Embedded SVG parsing | This version parses a nested SVG before removing nested image nodes; that later removal cannot protect against a file read during parsing. Regression cases cover nested img, background, mask and raw-SVG data. |
| SVG styles, imports, font-face URLs, presentation attributes, `use`, XML base/DTD/processing instructions | References are checked across the parsed document and CSS. External references and resource declarations fail closed. Internal paint/use fragments and ordinary SVG remain usable. |
| [Fonts](https://github.com/yisibl/resvg-js/blob/v2.6.2/src/fonts.rs) | `kit/raster.ts` supplies only the two bundled Inter files, no font directories, with system fonts disabled. Card output cannot change these options. |
| HTTP image placeholders / `imagesToResolve`, `resolveImage` | The binding exposes deferred resolution; this app never calls it. External references are refused before construction anyway. |

Tests live in `test/plugins/resources.test.ts`, `card.test.ts` and `author.test.ts`.
They include real hosted subprocess refusal and checker failure, valid embedded
assets, shared byte/count limits, and actual output dimensions. Accepted SVG text
is not rewritten; built-in golden SVGs retain their byte-exact gate.
