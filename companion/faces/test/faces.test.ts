import { describe, expect, test } from "bun:test";
import { ConfigurationError, TransientError } from "../src/face";
import { fetchHackerNews, renderHackerNews, storyFromItem } from "../src/faces/hackernews";
import { fetchRss, parseFeed, renderRss } from "../src/faces/rss";
import {
  coinFromSearch,
  fetchToken,
  parseCandles,
  parseMarkets,
  renderToken,
  splitPrice,
} from "../src/faces/token";
import { conditionFromWmo, fetchWeather, parseForecast } from "../src/faces/weather";
import type { FetchText } from "../src/kit/http";
import { pngFromSvg, textInk } from "../src/kit/raster";
import { BAD, GOOD } from "../src/kit/theme";

const NOW = new Date("2026-09-12T14:00:00Z");

/** A fetcher that answers from a table and records what it was asked for. */
function fake(routes: Record<string, string | Error>): FetchText & { asked: string[] } {
  const asked: string[] = [];
  const get = async (url: string): Promise<string> => {
    asked.push(url);
    const key = Object.keys(routes).find((fragment) => url.includes(fragment));
    const answer = key === undefined ? new TransientError(`no route for ${url}`) : routes[key];
    if (answer instanceof Error || answer === undefined) {
      throw answer ?? new TransientError("unrouted");
    }
    return answer;
  };
  return Object.assign(get, { asked });
}

const rendersToAFrame = (svg: string): boolean => pngFromSvg(svg).length > 1_000;

describe("token", () => {
  const MARKETS = JSON.stringify([
    {
      symbol: "sol",
      name: "Solana",
      current_price: 142.37,
      high_24h: 144.12,
      low_24h: 136.9,
      price_change_percentage_24h: 2.41,
    },
  ]);
  const CHART = JSON.stringify({
    prices: [
      [1, 100],
      [2, 101],
      [3, 102],
      [4, 103.5],
    ],
  });
  const quoted = (body: string, currency: string) => {
    const face = parseMarkets(body, currency);
    if (face === undefined) {
      throw new Error("the fixture lists a coin");
    }
    return face;
  };

  test("a quote and its chart are read into one face", async () => {
    const get = fake({ "/coins/markets": MARKETS, "/market_chart": CHART });
    const face = await fetchToken({ coin_id: "solana", currency: "usd" }, get);
    expect(face).toMatchObject({
      symbol: "sol",
      name: "Solana",
      currency: "USD",
      currencyMark: "$",
    });
    expect(face.series).toEqual([100, 101, 102, 103.5]);
    expect(get.asked[0]).toContain("vs_currency=usd&ids=solana&price_change_percentage=24h");
    expect(get.asked[1]).toContain("/coins/solana/market_chart?vs_currency=usd&days=1");
  });

  test("a chart failure still publishes the quote, with no sparkline", async () => {
    // A rate-limited decoration must not replace a correct price with a stale one.
    const get = fake({ "/coins/markets": MARKETS, "/market_chart": new TransientError("429") });
    const face = await fetchToken({ coin_id: "solana" }, get);
    expect(face.price).toBe(142.37);
    expect(face.series).toEqual([]);
    expect(rendersToAFrame(renderToken(face))).toBe(true);
  });

  const SEARCH = JSON.stringify({
    coins: [
      { id: "solv-protocol", symbol: "SOLV", name: "Solv Protocol", market_cap_rank: 807 },
      { id: "wrapped-solana", symbol: "SOL", name: "Wrapped SOL", market_cap_rank: 412 },
      { id: "solana", symbol: "SOL", name: "Solana", market_cap_rank: 7 },
    ],
  });

  test("a ticker or a name is resolved to the coin the owner meant", async () => {
    // What people type. CoinGecko answers `ids=sol` with [], and until this existed
    // that was a log line on the VM and "Waiting for the first picture" on the panel.
    for (const typed of ["SOL", "sol", "Solana", "  solana  "]) {
      const get = fake({
        "ids=solana&": MARKETS,
        "/coins/markets": "[]",
        "/search": SEARCH,
        "/market_chart": CHART,
      });
      const face = await fetchToken({ coin_id: typed }, get);
      expect(face.name).toBe("Solana");
      expect(get.asked.at(-1)).toContain("/coins/solana/market_chart");
    }
  });

  test("the id the placeholder shows costs one quote request and no search", async () => {
    const get = fake({ "/coins/markets": MARKETS, "/market_chart": CHART });
    await fetchToken({ coin_id: "solana" }, get);
    expect(get.asked.filter((url) => url.includes("/search"))).toEqual([]);
    expect(get.asked).toHaveLength(2);
  });

  test("only an exact match is accepted, best-ranked first: SOL never becomes Solv Protocol", () => {
    expect(coinFromSearch(SEARCH, "SOL")?.id).toBe("solana");
    expect(coinFromSearch(SEARCH, "solv protocol")?.id).toBe("solv-protocol");
    expect(coinFromSearch(SEARCH, "solv")?.id).toBe("solv-protocol");
    expect(coinFromSearch(SEARCH, "sola")).toBeUndefined();
    expect(
      coinFromSearch('{"coins":[{"id":"../x","symbol":"SOL","name":"x"}]}', "sol"),
    ).toBeUndefined();
  });

  test("a coin nobody lists is the owner's to fix, and says what to try", async () => {
    const get = fake({ "/coins/markets": "[]", "/search": '{"coins":[]}' });
    const failure = await fetchToken({ coin_id: "solanna" }, get).catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(ConfigurationError);
    expect((failure as Error).message).toContain('no coin called "solanna"');
  });

  test("an empty coin and one that is not a name at all are refused before any request", async () => {
    for (const coin_id of ["", "   ", "../etc/passwd", "<script>", "x".repeat(65)]) {
      const get = fake({});
      await expect(fetchToken({ coin_id }, get)).rejects.toBeInstanceOf(ConfigurationError);
      expect(get.asked).toEqual([]);
    }
  });

  test("a hand-edited api key rides on both requests and an absent one adds nothing", async () => {
    const keyed = fake({ "/coins/markets": MARKETS, "/market_chart": CHART });
    await fetchToken({ coin_id: "solana", api_key: "CG-secret" }, keyed);
    expect(keyed.asked.every((url) => url.includes("x_cg_demo_api_key=CG-secret"))).toBe(true);
    const bare = fake({ "/coins/markets": MARKETS, "/market_chart": CHART });
    await fetchToken({ coin_id: "solana", api_key: null }, bare);
    expect(bare.asked.some((url) => url.includes("x_cg_demo_api_key"))).toBe(false);
  });

  test("an inverted or missing range falls back rather than drawing nonsense", () => {
    const face = quoted(JSON.stringify([{ current_price: 5, high_24h: 4, low_24h: 6 }]), "eur");
    expect([face.low, face.high]).toEqual([4, 6]);
    expect(face.currencyMark).toBe("€");
    const bare = quoted(JSON.stringify([{ current_price: 5 }]), "xyz");
    expect([bare.low, bare.high, bare.changePercent, bare.currencyMark]).toEqual([5, 5, 0, ""]);
  });

  test("a fall prints its sign as well as its colour", () => {
    const face = quoted(MARKETS, "usd");
    const falling = renderToken({ ...face, changePercent: -3.08 });
    expect(falling).toContain("\u22123.08%");
    expect(falling).toContain(BAD);
    expect(renderToken(face)).toContain("+2.41%");
    expect(renderToken(face)).toContain(GOOD);
  });

  test("candles ask the ohlc endpoint instead of the line's, never both", async () => {
    const OHLC = JSON.stringify([
      [1, 108.06, 108.2, 107.95, 108.03],
      [2, 108.01, 108.24, 107.93, 108.11],
    ]);
    const get = fake({ "/coins/markets": MARKETS, "/ohlc": OHLC });
    const face = await fetchToken({ coin_id: "solana", chart: "candles" }, get);
    expect(face.chart).toBe("candles");
    expect(face.candles).toEqual([
      { open: 108.06, high: 108.2, low: 107.95, close: 108.03 },
      { open: 108.01, high: 108.24, low: 107.93, close: 108.11 },
    ]);
    expect(get.asked.some((url) => url.includes("market_chart"))).toBe(false);
    expect(get.asked.at(-1)).toContain("/coins/solana/ohlc?vs_currency=usd&days=1");
    // An unknown choice is the default, not an error: the field is an enum upstream.
    expect(
      (await fetchToken({ coin_id: "solana", chart: "renko" }, fake({ "/coins/markets": MARKETS })))
        .chart,
    ).toBe("line");
  });

  test("a refused candle request leaves a true price over an empty chart", async () => {
    const get = fake({ "/coins/markets": MARKETS, "/ohlc": new TransientError("429") });
    const face = await fetchToken({ coin_id: "solana", chart: "candles" }, get);
    expect(face.price).toBe(142.37);
    expect(face.candles).toEqual([]);
    expect(rendersToAFrame(renderToken(face))).toBe(true);
  });

  test("a malformed candle is dropped and an inverted one cannot draw a wick inside out", () => {
    const candles = parseCandles(
      JSON.stringify([[1, 10, 9, 12, 11], [2, 10, "x", 9, 11], [3, 10], "junk", [4, 5, 6, 4, 5]]),
    );
    expect(candles).toEqual([
      { open: 10, high: 12, low: 9, close: 11 },
      { open: 5, high: 6, low: 4, close: 5 },
    ]);
  });

  test("each candle is coloured by its own half hour, whatever the day did", () => {
    const svg = renderToken({
      ...quoted(MARKETS, "usd"),
      chart: "candles",
      candles: [
        { open: 10, high: 12, low: 9, close: 11 },
        { open: 11, high: 11.5, low: 8, close: 9 },
      ],
    });
    expect(svg).toContain(GOOD);
    expect(svg).toContain(BAD);
  });

  test("the price starts at the module's margin by its ink, whatever digit leads it", () => {
    // An advance-based layout put the first glyph a bearing inside the margin, so the
    // price sat right of the symbol above it. The mark's INK must start at the margin
    // at every size the price is fitted to.
    for (const price of [118.04, 842.5, 104235.5, 0.004182]) {
      const svg = renderToken({ ...quoted(MARKETS, "usd"), price });
      const mark = svg.match(/<text x="([\d.]+)"[^>]*font-size="([\d.]+)"[^>]*>\$<\/text>/);
      const inkLeft = Number(mark?.[1]) + textInk("$", Number(mark?.[2]), 600).left;
      expect(inkLeft).toBeCloseTo(24 + 20, 1);
    }
  });

  test("decimals follow the magnitude, and only high precision is trimmed", () => {
    expect(splitPrice(104235.5)).toEqual(["104,235", ".50"]);
    expect(splitPrice(142.5)).toEqual(["142", ".50"]);
    expect(splitPrice(0.0371)).toEqual(["0", ".0371"]);
    expect(splitPrice(0.0000371)).toEqual(["0", ".0000371"]);
    expect(splitPrice(0)).toEqual(["0", ".00"]);
  });
});

describe("rss", () => {
  const RSS = `<?xml version="1.0"?><rss version="2.0"><channel><title>Feed</title>
    <item><title>First &amp; foremost</title><pubDate>Sat, 12 Sep 2026 13:46:00 GMT</pubDate></item>
    <item><title><![CDATA[Second <b>story</b>]]></title><pubDate>not a date</pubDate></item>
    <item><title>Third</title></item><item><title>Fourth</title></item><item><title>Fifth</title></item>
  </channel></rss>`;
  const ATOM = `<feed xmlns="http://www.w3.org/2005/Atom"><title>Releases</title>
    <entry><title type="html">v1.4 &lt;em&gt;out&lt;/em&gt;</title><updated>2026-09-12T11:00:00Z</updated></entry>
  </feed>`;

  test("RSS 2.0 parses, entities and CDATA included, capped at what the face shows", () => {
    const entries = parseFeed(RSS, NOW);
    expect(entries.map((entry) => entry.title)).toEqual([
      "First & foremost",
      "Second story",
      "Third",
      "Fourth",
    ]);
    expect(entries.map((entry) => entry.age)).toEqual(["14m", "", "", ""]);
  });

  test("Atom parses, with an html title reduced to its text", () => {
    expect(parseFeed(ATOM, NOW)).toEqual([{ title: "v1.4 out", age: "3h" }]);
  });

  test("a page that is not a feed sends the owner back to the URL", async () => {
    const get = fake({ "example.com": "<html><body>hello</body></html>" });
    const settings = { url: "https://example.com/", title: "News" };
    expect(fetchRss(settings, NOW, get)).rejects.toBeInstanceOf(ConfigurationError);
  });

  test("a hostile headline cannot escape its text node, and still renders", () => {
    const svg = renderRss({
      feedTitle: "News",
      entries: [{ title: "</text><script>fetch('http://evil.test')</script><text>", age: "1h" }],
    });
    expect(svg).not.toContain("<script>");
    expect(svg).toContain("&lt;script&gt;");
    expect(rendersToAFrame(svg)).toBe(true);
  });

  test("an item without a date omits its age instead of inventing one", () => {
    const svg = renderRss({ feedTitle: "Undated", entries: [{ title: "A story", age: "" }] });
    expect(svg).not.toMatch(/just now|ago/);
  });
});

describe("weather", () => {
  const GEOCODING = JSON.stringify({
    results: [
      { name: "Dubai", country: "United Arab Emirates", latitude: 25.07, longitude: 55.17 },
    ],
  });
  const forecast = (current: object, hourly?: object): string =>
    JSON.stringify({
      current: {
        time: "2026-09-12T14:15",
        temperature_2m: 33.5,
        weather_code: 1,
        is_day: 1,
        ...current,
      },
      daily: { temperature_2m_max: [38.4], temperature_2m_min: [27.2] },
      hourly: hourly ?? {
        time: ["2026-09-12T13:00", "2026-09-12T14:00", "2026-09-12T15:00", "2026-09-12T16:00"],
        temperature_2m: [33, 34.4, 35.5, null],
        weather_code: [0, 1, 3, 61],
        is_day: [1, 1, 0, 0],
      },
    });

  test("the face says what the forecast is actually for, in the geocoder's words", async () => {
    const get = fake({ "geocoding-api": GEOCODING, "/v1/forecast": forecast({}) });
    const face = await fetchWeather({ location: "dubai", units: "metric" }, get);
    expect(face.place).toBe("Dubai, United Arab Emirates");
    expect(get.asked[1]).toContain("latitude=25.07&longitude=55.17");
    expect(get.asked[1]).toContain("timezone=auto");
    expect(get.asked[1]).not.toContain("temperature_unit");
  });

  test("imperial asks the API for fahrenheit rather than converting", async () => {
    const get = fake({ "geocoding-api": GEOCODING, "/v1/forecast": forecast({}) });
    await fetchWeather({ location: "Dubai", units: "imperial" }, get);
    expect(get.asked[1]).toContain("temperature_unit=fahrenheit");
  });

  test("a place nobody has heard of is a setting to fix", async () => {
    const get = fake({ "geocoding-api": "{}" });
    expect(fetchWeather({ location: "Qwertyuiop" }, get)).rejects.toBeInstanceOf(
      ConfigurationError,
    );
  });

  test("the strip starts at the current local hour and skips an unusable slot", () => {
    const face = parseForecast(forecast({}), "Dubai");
    expect(face.hourly).toEqual([
      { label: "14", temperature: 34, condition: "clear-day" },
      { label: "15", temperature: 36, condition: "cloudy" },
    ]);
    expect([face.temperature, face.high, face.low]).toEqual([34, 38, 27]);
  });

  test("halves round away from zero in both directions", () => {
    expect(parseForecast(forecast({ temperature_2m: 18.5 }), "x").temperature).toBe(19);
    expect(parseForecast(forecast({ temperature_2m: -18.5 }), "x").temperature).toBe(-19);
    expect(parseForecast(forecast({ temperature_2m: -0.4 }), "x").temperature).toBe(0);
  });

  test("a forecast with no usable hourly block still draws, with an empty strip", () => {
    expect(parseForecast(forecast({}, {}), "Dubai").hourly).toEqual([]);
    expect(parseForecast(forecast({ time: "garbage" }), "Dubai").hourly).toEqual([]);
  });

  test("conditions are coarser than the WMO list, and night is a different face", () => {
    expect(conditionFromWmo(0, true)).toBe("clear-day");
    expect(conditionFromWmo(1, false)).toBe("clear-night");
    expect(conditionFromWmo(2, false)).toBe("partly-cloudy-night");
    expect(conditionFromWmo(3, true)).toBe("cloudy");
    expect(conditionFromWmo(67, true)).toBe("sleet");
    expect(conditionFromWmo(82, true)).toBe("rain");
    expect(conditionFromWmo(99, false)).toBe("thunderstorm");
    expect(conditionFromWmo(12345, true)).toBe("cloudy");
  });
});

describe("hacker news", () => {
  const item = (id: number, extra: object = {}): string =>
    JSON.stringify({
      id,
      type: "story",
      title: `Story ${id}`,
      score: id * 10,
      descendants: id,
      url: `https://www.example${id}.com/post`,
      time: NOW.getTime() / 1000 - 3 * 3600,
      ...extra,
    });

  test("stories arrive in list order with the www stripped and the age rendered", async () => {
    const get = fake({
      "topstories.json": "[1, 2, 3, 4, 5, 6, 7, 8]",
      "item/1.json": item(1),
      "item/2.json": item(2),
      "item/3.json": item(3),
      "item/4.json": item(4),
      "item/5.json": item(5),
      "item/6.json": item(6),
    });
    const face = await fetchHackerNews({}, NOW, get);
    expect(face.stories.map((story) => story.rank)).toEqual([1, 2, 3, 4]);
    expect(face.stories[0]).toEqual({
      rank: 1,
      title: "Story 1",
      points: 10,
      comments: 1,
      domain: "example1.com",
      age: "3h",
    });
    expect(get.asked.some((url) => url.includes("item/7.json"))).toBe(false);
  });

  test("a dead, deleted or unreadable story is skipped and the ranks stay contiguous", async () => {
    const get = fake({
      "beststories.json": "[1, 2, 3, 4, 5, 6]",
      "item/1.json": item(1, { dead: true }),
      "item/2.json": item(2),
      "item/3.json": new TransientError("timeout"),
      "item/4.json": item(4, { deleted: true }),
      "item/5.json": item(5),
      "item/6.json": "null",
    });
    const face = await fetchHackerNews({ list: "best" }, NOW, get);
    expect(face.stories.map((story) => [story.rank, story.title])).toEqual([
      [1, "Story 2"],
      [2, "Story 5"],
    ]);
  });

  test("when no story at all can be read the stored frame is kept", async () => {
    const get = fake({ "topstories.json": "[1, 2]" });
    expect(fetchHackerNews({}, NOW, get)).rejects.toBeInstanceOf(TransientError);
  });

  test("an Ask HN post has no domain, and entities in a title are decoded once", () => {
    const story = storyFromItem(
      { title: "Ask HN: Who&#x27;s hiring &amp; where?", score: 5, time: 0 },
      1,
      NOW,
    );
    expect(story).toMatchObject({
      title: "Ask HN: Who's hiring & where?",
      domain: "",
      comments: 0,
    });
  });

  const story = (rank: number, title: string) => ({
    rank,
    title,
    points: 4321,
    comments: 1234,
    domain: "example.com",
    age: "2h",
  });

  test("a long headline costs index rows before it costs a word", () => {
    const long = "Human brain is two separate organs, Stanford Medicine-led research finds";
    const svg = renderHackerNews({
      stories: [story(1, long), story(2, long), story(3, long), story(4, long)],
    });
    expect(svg).not.toContain("…");
    // Every word of the lead is on the face.
    for (const word of long.split(" ")) {
      expect(svg).toContain(word.replace("&", "&amp;"));
    }
  });

  test("a short front page keeps its index: the lone lead is a fallback, not a preference", () => {
    const svg = renderHackerNews({
      stories: [story(1, "OpenJev"), story(2, "Cloudflare Quick Tunnels"), story(3, "Bun 1.4")],
    });
    expect(svg).toContain("Cloudflare Quick Tunnels");
    expect(svg).toContain("Bun 1.4");
  });

  test("a pathological title is cut inside the module rather than overflowing it", () => {
    const svg = renderHackerNews({ stories: [story(1, "Pneumonoultramicroscopic".repeat(12))] });
    expect(svg).toContain("…");
    expect(rendersToAFrame(svg)).toBe(true);
    for (const match of svg.matchAll(/<text x="[\d.-]+" y="([\d.-]+)"/g)) {
      expect(Number(match[1])).toBeLessThanOrEqual(368 - 24);
    }
  });
});
