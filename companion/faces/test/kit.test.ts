import { describe, expect, test } from "bun:test";
import { relativeAge } from "../src/kit/age";
import {
  Canvas,
  escapeXml,
  fit,
  fitSize,
  fitTracked,
  fixed,
  textWidth,
  trackedWidth,
  wrap,
} from "../src/kit/svg";
import { HERO_STEPS } from "../src/kit/theme";

describe("fixed", () => {
  test("rounds exact ties to even, as the Rust `{:.N}` the goldens came from does", () => {
    // n/8 is exactly representable, and 8px grid arithmetic produces it.
    expect(fixed(0.125, 2)).toBe("0.12");
    expect(fixed(0.375, 2)).toBe("0.38");
    expect(fixed(0.625, 2)).toBe("0.62");
    expect(fixed(2.5, 0)).toBe("2");
    expect(fixed(3.5, 0)).toBe("4");
  });

  test("leaves everything that is not a tie to ordinary rounding", () => {
    expect(fixed(85.333984375, 2)).toBe("85.33");
    expect(fixed(10.905, 2)).toBe("10.90");
    expect(fixed(1234.565, 2)).toBe("1234.57");
    expect(fixed(448, 0)).toBe("448");
  });
});

describe("escapeXml", () => {
  test("neutralizes every predefined entity", () => {
    expect(escapeXml(`<a href="x">Tom & 'Jerry'</a>`)).toBe(
      "&lt;a href=&quot;x&quot;&gt;Tom &amp; &apos;Jerry&apos;&lt;/a&gt;",
    );
  });

  test("drops the control characters that would make the document unparseable", () => {
    expect(escapeXml("a\u0000b\u0007c\td")).toBe("abc d");
  });
});

describe("text fitting", () => {
  test("measurement is zero for blank text and grows with length and size", () => {
    expect(textWidth("   ", 24, 400)).toBe(0);
    const short = textWidth("Solana", 24, 600);
    expect(short).toBeGreaterThan(0);
    expect(textWidth("Solana network fees", 24, 600)).toBeGreaterThan(short);
    expect(textWidth("Solana", 48, 600)).toBeGreaterThan(short * 1.5);
  });

  test("a headline wraps within its measured width", () => {
    const lines = wrap(
      "Rust 1.98 stabilises const generics and ships a faster linker",
      31,
      600,
      380,
      3,
    );
    expect(lines.length).toBeGreaterThan(1);
    for (const line of lines) {
      expect(textWidth(line, 31, 600)).toBeLessThanOrEqual(380);
    }
  });

  test("overlong text ellipsizes on the last allowed line", () => {
    const lines = wrap("word ".repeat(80), 31, 600, 380, 2);
    expect(lines).toHaveLength(2);
    expect(lines[1]?.endsWith("…")).toBe(true);
    expect(textWidth(lines[1] ?? "", 31, 600)).toBeLessThanOrEqual(380);
  });

  test("an unbreakable word is split rather than overhanging", () => {
    // One split is only enough for a word under twice the box width; pushing the
    // unmeasured remainder is how a compound noun overhung with every test green.
    const lines = wrap("Donaudampfschiffahrtsgesellschaftskapitaensversammlung", 31, 600, 200, 5);
    expect(lines.length).toBeGreaterThan(2);
    for (const line of lines) {
      expect(textWidth(line, 31, 600)).toBeLessThanOrEqual(200);
    }
  });

  test("blank text and a zero line budget yield nothing", () => {
    expect(wrap("   ", 21, 400, 100, 3)).toEqual([]);
    expect(wrap("text", 21, 400, 100, 0)).toEqual([]);
  });

  test("fit keeps text that fits and marks text that does not", () => {
    expect(fit("Short", 21, 400, 400)).toBe("Short");
    const cut = fit("A postmortem of the eu-west-1 control plane outage", 21, 400, 200);
    expect(cut.endsWith("…")).toBe(true);
    expect(textWidth(cut, 21, 400)).toBeLessThanOrEqual(200);
  });

  test("tracking widens a run by one step per character, and fitting respects it", () => {
    const place = "DUBAI, UNITED ARAB EMIRATES";
    expect(trackedWidth(place, 15, 600, 1.4)).toBeCloseTo(
      textWidth(place, 15, 600) + 1.4 * place.length,
      6,
    );
    // The trap this exists for: it measures as fitting and draws ~38px wider.
    const room = textWidth(place, 15, 600) + 10;
    const fitted = fitTracked(place, 15, 600, 1.4, room);
    expect(fitted.endsWith("…")).toBe(true);
    expect(trackedWidth(fitted, 15, 600, 1.4)).toBeLessThanOrEqual(room);
  });

  test("fitSize picks the largest candidate that fits and falls back to the last", () => {
    expect(fitSize("34°", 600, 260, HERO_STEPS)).toBe(112);
    expect(fitSize("-118°", 600, 200, HERO_STEPS)).toBeLessThan(112);
    expect(fitSize("x".repeat(200), 600, 10, HERO_STEPS)).toBe(60);
  });
});

test("a finished canvas is one well-formed root that escapes its text", () => {
  const canvas = new Canvas(448, 368);
  canvas.rect(0, 0, 448, 368, "#000000");
  canvas.text({ x: 24, baseline: 40, content: "</text><script>", size: 21, fill: "#fff" });
  const svg = canvas.finish();
  expect(svg.startsWith('<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368"')).toBe(
    true,
  );
  expect(svg.endsWith("</svg>")).toBe(true);
  expect(svg).not.toContain("<script>");
  expect(svg).toContain("&lt;script&gt;");
});

describe("relativeAge", () => {
  const now = new Date("2026-09-12T12:00:00Z");
  const ago = (hours: number, minutes = 0): Date =>
    new Date(now.getTime() - (hours * 60 + minutes) * 60_000);

  test("steps up through coarser units", () => {
    expect(relativeAge(ago(0), now)).toBe("now");
    expect(relativeAge(ago(0, 14), now)).toBe("14m");
    expect(relativeAge(ago(3), now)).toBe("3h");
    expect(relativeAge(ago(30), now)).toBe("1d");
    expect(relativeAge(ago(24 * 10), now)).toBe("1w");
    expect(relativeAge(ago(24 * 364), now)).toBe("52w");
    expect(relativeAge(ago(24 * 365), now)).toBe("1y");
  });

  test("an item dated in the future reads as now, not as a negative age", () => {
    expect(relativeAge(new Date(now.getTime() + 5 * 3_600_000), now)).toBe("now");
  });
});
