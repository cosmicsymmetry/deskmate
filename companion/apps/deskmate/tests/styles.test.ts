import { expect, test } from "bun:test";

test("includes narrow-viewport and reduced-motion fallbacks", async () => {
  const css = await Bun.file(new URL("../src/styles.css", import.meta.url)).text();
  expect(css).toContain("@media (max-width: 430px)");
  expect(css).toContain("@media (prefers-reduced-motion: reduce)");
  expect(css).toContain("animation-duration: 0.001ms");
});
