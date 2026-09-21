// The review set: every face against the inputs that have actually broken one.
//
// `bun run dump` writes these as PNGs for a human to look at; `test/golden.test.ts`
// pins each one's SVG byte for byte. A golden SVG is this package's own
// deterministic output, so unlike a golden PNG it does not move when the rasterizer
// is upgraded -- it moves only when a face's geometry does, which is the thing worth
// being told about. The weather, RSS and token goldens are the Rust renderer's own
// output from before the move to TypeScript: the port was accepted on being
// byte-identical to them.

import { type HackerNewsFace, renderHackerNews } from "./faces/hackernews";
import { type FeedEntry, renderRss } from "./faces/rss";
import { type Candle, renderToken, type TokenFace } from "./faces/token";
import { type Condition, type HourlyStep, renderWeather } from "./faces/weather";

export interface Case {
  name: string;
  svg: string;
}

const entry = (title: string, age: string): FeedEntry => ({ title, age });
const hour = (label: string, temperature: number, condition: Condition): HourlyStep => ({
  label,
  temperature,
  condition,
});

/**
 * A plausible 24-hour price series with a visible trend and some noise, so the
 * sparkline is reviewed against something shaped like real data, not a sine wave.
 */
function series(start: number, drift: number, samples: number): number[] {
  return Array.from({ length: samples }, (_, index) => {
    const progress = index / (samples - 1);
    const wobble = Math.sin(index / 6) * 1.4 + Math.cos(index / 2.3) * 0.6;
    return start + drift * progress + wobble;
  });
}

const sol: Omit<TokenFace, "price" | "changePercent" | "low" | "high" | "series"> = {
  symbol: "SOL",
  name: "Solana",
  currency: "USD",
  currencyMark: "$",
  candles: [],
  chart: "line",
  coinId: "solana",
};

// One real day of SOL/USD half-hour candles (CoinGecko `ohlc`, captured 2026-09-21):
// open, high, low, close. Real candles have dojis, gaps and one-sided wicks; invented
// ones are all tidy, which is how a two-pixel body never gets looked at.
const SOL_DAY: [number, number, number, number][] = [
  [108.06, 108.2, 107.95, 108.03],
  [108.01, 108.24, 107.93, 108.11],
  [108.11, 108.58, 108.11, 108.46],
  [108.44, 108.74, 108.19, 108.19],
  [108.19, 108.38, 108.12, 108.35],
  [108.35, 108.49, 108.14, 108.19],
  [108.19, 108.2, 107.98, 108.17],
  [108.17, 108.54, 107.98, 108.12],
  [108.12, 108.35, 108.11, 108.12],
  [108.12, 108.41, 108.07, 108.07],
  [108.07, 108.75, 108.07, 108.75],
  [108.71, 109.39, 108.51, 109.36],
  [109.29, 110.57, 109.29, 110.21],
  [110.23, 110.34, 109.56, 109.62],
  [109.62, 110.43, 109.6, 109.88],
  [109.87, 110.27, 109.65, 110.25],
  [110.27, 110.3, 109.27, 109.4],
  [109.4, 109.96, 109.35, 109.92],
  [109.93, 110.74, 109.93, 110.43],
  [110.46, 110.54, 110.19, 110.46],
  [110.37, 110.55, 110.1, 110.11],
  [110.11, 110.11, 109.3, 109.88],
  [109.86, 109.99, 109.6, 109.84],
  [109.84, 110.51, 109.67, 110.47],
  [110.48, 110.99, 110.31, 110.96],
  [110.96, 110.97, 110.61, 110.68],
  [110.7, 111.31, 110.69, 111.13],
  [111.15, 112.74, 111.15, 112.36],
  [112.3, 112.7, 112.09, 112.3],
  [112.29, 113.17, 112.08, 112.33],
  [112.32, 112.59, 110.89, 110.98],
  [110.98, 111.91, 110.92, 111.73],
  [111.73, 111.73, 110.99, 111.19],
  [111.19, 111.78, 111.17, 111.57],
  [111.57, 111.82, 111.5, 111.53],
  [111.54, 111.57, 111.11, 111.17],
  [111.16, 111.72, 111.05, 111.45],
  [111.45, 111.96, 111.29, 111.92],
  [111.88, 112.11, 111.69, 111.75],
  [111.76, 112.43, 111.7, 112.37],
  [112.39, 112.42, 111.57, 112.14],
  [112.14, 112.32, 111.7, 111.74],
  [111.74, 112.04, 111.41, 112.03],
  [112.08, 113.51, 112.08, 113.49],
  [113.48, 116.09, 113.43, 115.3],
  [115.29, 115.92, 114.98, 115.92],
  [115.93, 116.62, 115.52, 115.86],
  [115.83, 116.0, 115.57, 115.74],
];
const candlesOf = (rows: [number, number, number, number][], factor = 1): Candle[] =>
  rows.map(([open, high, low, close]) => ({
    open: open * factor,
    high: high * factor,
    low: low * factor,
    close: close * factor,
  }));

function rssCases(): Case[] {
  return [
    {
      name: "rss--three-line-lead",
      svg: renderRss({
        feedTitle: "Hacker News",
        entries: [
          entry("Rust 1.98 stabilises const generics and ships a much faster linker", "14m"),
          entry("A postmortem of the eu-west-1 control plane outage", "1h"),
          entry("Writing a toy TCP stack in 400 lines of Zig", "3h"),
          entry("The case against microservices, revisited", "5h"),
        ],
      }),
    },
    {
      name: "rss--one-line-lead",
      svg: renderRss({
        feedTitle: "Changelog",
        entries: [
          entry("Postgres 19 is out", "6m"),
          entry("SQLite adds strict table checks by default", "2h"),
          entry("Zig 0.16 release notes", "8h"),
          entry("A new ARM backend for LLVM", "1d"),
        ],
      }),
    },
    { name: "rss--empty-feed", svg: renderRss({ feedTitle: "Quiet feed", entries: [] }) },
    {
      name: "rss--single-item",
      svg: renderRss({ feedTitle: "Status", entries: [entry("All systems operational", "30m")] }),
    },
    {
      name: "rss--undated-items",
      svg: renderRss({
        feedTitle: "No dates",
        entries: [
          entry("A feed whose items carry no pubDate at all", ""),
          entry("So none of these rows print an age", ""),
          entry("And the lead prints none either", ""),
        ],
      }),
    },
    {
      // One unbreakable word wider than the headline box: it must be split, not overhang.
      name: "rss--prefixed-compound-lead",
      svg: renderRss({
        feedTitle: "News",
        entries: [entry("A Donaudampfschiffahrtsgesellschaftskapitaensversammlung", "1h")],
      }),
    },
  ];
}

function tokenCases(): Case[] {
  return [
    {
      name: "token--solana-rising",
      svg: renderToken({
        ...sol,
        price: 142.37,
        changePercent: 2.41,
        low: 136.9,
        high: 144.12,
        series: series(137, 5.2, 96),
      }),
    },
    {
      name: "token--solana-falling",
      svg: renderToken({
        ...sol,
        price: 118.04,
        changePercent: -6.83,
        low: 116.5,
        high: 127.8,
        series: series(127, -9, 96),
      }),
    },
    {
      name: "token--five-figure-price",
      svg: renderToken({
        ...sol,
        symbol: "BTC",
        name: "Bitcoin",
        price: 104_235.5,
        changePercent: 0.42,
        low: 103_010,
        high: 105_700,
        series: series(103_500, 700, 96),
      }),
    },
    {
      name: "token--sub-cent-price",
      svg: renderToken({
        ...sol,
        symbol: "BONK",
        name: "Bonk",
        price: 0.000_041_82,
        changePercent: 11.6,
        low: 0.000_037_1,
        high: 0.000_043_9,
        series: series(0.000_037, 0.000_005, 96),
      }),
    },
    {
      name: "token--candles-real-day",
      svg: renderToken({
        ...sol,
        chart: "candles",
        price: SOL_DAY.at(-1)?.[3] ?? 0,
        changePercent: 7.12,
        low: Math.min(...SOL_DAY.map((row) => row[2])),
        high: Math.max(...SOL_DAY.map((row) => row[1])),
        series: [],
        candles: candlesOf(SOL_DAY),
      }),
    },
    {
      // The same shape at a sub-cent scale and with a long name: the price is set at
      // one size, and the range labels are at their widest.
      name: "token--candles-sub-cent",
      svg: renderToken({
        ...sol,
        symbol: "BONK",
        name: "Bonk Inu Community Token",
        chart: "candles",
        price: 0.000_041_82,
        changePercent: 11.6,
        low: 0.000_037_1,
        high: 0.000_043_9,
        series: [],
        candles: candlesOf(SOL_DAY, 0.000_000_385),
      }),
    },
    {
      name: "token--candles-unavailable",
      svg: renderToken({
        ...sol,
        chart: "candles",
        price: 142.37,
        changePercent: 2.41,
        low: 136.9,
        high: 144.12,
        series: [],
      }),
    },
    {
      name: "token--simple-rising",
      svg: renderToken({
        ...sol,
        chart: "none",
        price: 142.37,
        changePercent: 2.41,
        low: 136.9,
        high: 144.12,
        series: [],
      }),
    },
    {
      name: "token--simple-falling-five-figures",
      svg: renderToken({
        ...sol,
        symbol: "BTC",
        name: "Bitcoin",
        chart: "none",
        price: 104_235.5,
        changePercent: -1.2,
        low: 103_010,
        high: 105_700,
        series: [],
      }),
    },
    {
      name: "token--simple-sub-cent",
      svg: renderToken({
        ...sol,
        symbol: "BONK",
        name: "Bonk",
        chart: "none",
        price: 0.000_041_82,
        changePercent: 11.6,
        low: 0.000_037_1,
        high: 0.000_043_9,
        series: [],
      }),
    },
    {
      name: "token--no-series-yet",
      svg: renderToken({
        ...sol,
        price: 142.37,
        changePercent: 0,
        low: 142.37,
        high: 142.37,
        series: [],
      }),
    },
  ];
}

function weatherCases(): Case[] {
  return [
    {
      name: "weather--clear-day",
      svg: renderWeather({
        place: "Dubai",
        temperature: 34,
        summary: "Mostly clear",
        condition: "clear-day",
        high: 38,
        low: 27,
        hourly: [
          hour("14", 34, "clear-day"),
          hour("15", 35, "clear-day"),
          hour("16", 34, "partly-cloudy-day"),
          hour("17", 32, "partly-cloudy-day"),
          hour("18", 30, "cloudy"),
          hour("19", 29, "clear-night"),
        ],
      }),
    },
    {
      name: "weather--rain",
      svg: renderWeather({
        place: "Amsterdam",
        temperature: 11,
        summary: "Moderate rain",
        condition: "rain",
        high: 13,
        low: 8,
        hourly: [
          hour("09", 11, "rain"),
          hour("10", 11, "rain"),
          hour("11", 12, "drizzle"),
          hour("12", 12, "cloudy"),
          hour("13", 13, "partly-cloudy-day"),
          hour("14", 12, "rain"),
        ],
      }),
    },
    {
      name: "weather--snow-sub-zero",
      svg: renderWeather({
        place: "Tromso",
        temperature: -18,
        summary: "Heavy snow",
        condition: "snow",
        high: -9,
        low: -24,
        hourly: [
          hour("06", -18, "snow"),
          hour("07", -17, "snow"),
          hour("08", -15, "sleet"),
          hour("09", -13, "cloudy"),
          hour("10", -11, "partly-cloudy-day"),
          hour("11", -9, "clear-day"),
        ],
      }),
    },
    {
      name: "weather--thunderstorm-night",
      svg: renderWeather({
        place: "Singapore",
        temperature: 27,
        summary: "Thunderstorms",
        condition: "thunderstorm",
        high: 31,
        low: 26,
        hourly: [
          hour("22", 27, "thunderstorm"),
          hour("23", 27, "rain"),
          hour("00", 26, "rain"),
          hour("01", 26, "drizzle"),
          hour("02", 26, "partly-cloudy-night"),
          hour("03", 26, "clear-night"),
        ],
      }),
    },
    {
      name: "weather--fog",
      svg: renderWeather({
        place: "San Francisco",
        temperature: 13,
        summary: "Depositing rime fog",
        condition: "fog",
        high: 17,
        low: 11,
        hourly: [
          hour("07", 13, "fog"),
          hour("08", 13, "fog"),
          hour("09", 14, "cloudy"),
          hour("10", 15, "partly-cloudy-day"),
          hour("11", 16, "clear-day"),
          hour("12", 17, "clear-day"),
        ],
      }),
    },
    {
      // The shape the live geocoder returns: "city, country", long enough to reach
      // the high/low beside it. A tracked eyebrow measured without its tracking drew
      // straight through it.
      name: "weather--long-place-and-range",
      svg: renderWeather({
        place: "Dubai, United Arab Emirates",
        temperature: 37,
        summary: "Clear",
        condition: "clear-night",
        high: 44,
        low: 29,
        hourly: [
          hour("19", 37, "clear-night"),
          hour("20", 37, "clear-night"),
          hour("21", 35, "clear-night"),
          hour("22", 34, "clear-night"),
          hour("23", 33, "clear-night"),
          hour("00", 32, "clear-night"),
        ],
      }),
    },
    {
      name: "weather--long-place-name",
      svg: renderWeather({
        place: "Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch",
        temperature: 9,
        summary: "Freezing drizzle and blowing snow later",
        condition: "sleet",
        high: 10,
        low: 4,
        hourly: [
          hour("12", 9, "sleet"),
          hour("13", 9, "sleet"),
          hour("14", 8, "rain"),
          hour("15", 7, "rain"),
          hour("16", 6, "cloudy"),
          hour("17", 5, "partly-cloudy-night"),
        ],
      }),
    },
    {
      name: "weather--no-hourly",
      svg: renderWeather({
        place: "Reykjavik",
        temperature: 4,
        summary: "Overcast",
        condition: "cloudy",
        high: 6,
        low: 1,
        hourly: [],
      }),
    },
  ];
}

// Real front-page data, captured 2026-09-19: titles from 7 to 79 characters. Every
// fixture title being short is how an overflow survives until a live response.
const hn = (stories: HackerNewsFace["stories"]): string => renderHackerNews({ stories });

function hackerNewsCases(): Case[] {
  return [
    {
      name: "hackernews--front-page",
      svg: hn([
        {
          rank: 1,
          title: "Android 17 is the first since 3.x to add new APIs without releasing to the AOSP",
          points: 991,
          comments: 554,
          domain: "grapheneos.social",
          age: "2h",
        },
        {
          rank: 2,
          title: "Cloudflare Quick Tunnels",
          points: 776,
          comments: 301,
          domain: "try.cloudflare.com",
          age: "5h",
        },
        {
          rank: 3,
          title: "AI-generated posters don’t have to be horrible",
          points: 775,
          comments: 466,
          domain: "john.hartnup.uk",
          age: "3h",
        },
        { rank: 4, title: "OpenJev", points: 670, comments: 280, domain: "openjev.com", age: "1d" },
      ]),
    },
    {
      name: "hackernews--long-titles-big-numbers",
      svg: hn([
        {
          rank: 1,
          title: "Android 17 is the first since 3.x to add new APIs without releasing to the AOSP",
          points: 3973,
          comments: 3216,
          domain: "grapheneos.social",
          age: "2h",
        },
        {
          rank: 2,
          title: "AI-generated posters don’t have to be horrible",
          points: 3325,
          comments: 2864,
          domain: "john.hartnup.uk",
          age: "3h",
        },
        {
          rank: 3,
          title: "Human brain is two separate organs, Stanford Medicine-led research finds",
          points: 2497,
          comments: 1732,
          domain: "med.stanford.edu",
          age: "7h",
        },
        {
          rank: 4,
          title: "The first new cat species discovered in 100 years",
          points: 1960,
          comments: 1496,
          domain: "nationalgeographic.com",
          age: "1d",
        },
      ]),
    },
    {
      name: "hackernews--short-titles",
      svg: hn([
        {
          rank: 1,
          title: "Cloudflare Quick Tunnels",
          points: 776,
          comments: 301,
          domain: "try.cloudflare.com",
          age: "5h",
        },
        { rank: 2, title: "OpenJev", points: 670, comments: 280, domain: "openjev.com", age: "1d" },
        {
          rank: 3,
          title: "How to Write with an LLM",
          points: 562,
          comments: 360,
          domain: "sockpuppet.org",
          age: "1d",
        },
        {
          rank: 4,
          title: "Saving another 100TB of RAM",
          points: 421,
          comments: 91,
          domain: "blog.cloudflare.com",
          age: "18h",
        },
      ]),
    },
    {
      name: "hackernews--ask-hn-lead",
      svg: hn([
        {
          rank: 1,
          title: "Ask HN: What are you working on? (September 2026)",
          points: 312,
          comments: 1204,
          domain: "",
          age: "4h",
        },
        {
          rank: 2,
          title: "Human brain is two separate organs, Stanford Medicine-led research finds",
          points: 499,
          comments: 183,
          domain: "med.stanford.edu",
          age: "7h",
        },
        {
          rank: 3,
          title: "Saving another 100TB of RAM",
          points: 421,
          comments: 91,
          domain: "blog.cloudflare.com",
          age: "18h",
        },
        {
          rank: 4,
          title: "The first new cat species discovered in 100 years",
          points: 320,
          comments: 124,
          domain: "nationalgeographic.com",
          age: "1d",
        },
      ]),
    },
    {
      name: "hackernews--fresh-low-numbers",
      svg: hn([
        { rank: 1, title: "OpenJev", points: 3, comments: 0, domain: "openjev.com", age: "now" },
        {
          rank: 2,
          title: "Laya the open source version of Jev",
          points: 1,
          comments: 0,
          domain: "laya.convaiinnovations.com",
          age: "2m",
        },
        {
          rank: 3,
          title: "How to Write with an LLM",
          points: 12,
          comments: 1,
          domain: "sockpuppet.org",
          age: "9m",
        },
        {
          rank: 4,
          title: "Human brain is two separate organs, Stanford Medicine-led research finds",
          points: 0,
          comments: 0,
          domain: "med.stanford.edu",
          age: "14m",
        },
      ]),
    },
    {
      name: "hackernews--single-story",
      svg: hn([
        {
          rank: 1,
          title: "Android 17 is the first since 3.x to add new APIs without releasing to the AOSP",
          points: 991,
          comments: 554,
          domain: "grapheneos.social",
          age: "2h",
        },
      ]),
    },
    { name: "hackernews--empty", svg: hn([]) },
  ];
}

export function allCases(): Case[] {
  return [...rssCases(), ...tokenCases(), ...weatherCases(), ...hackerNewsCases()];
}
