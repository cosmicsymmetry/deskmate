export interface NowContext {
  utc: string;
  timezone: string;
  local: {
    year: number;
    month: number;
    day: number;
    hour: number;
    minute: number;
    weekday: string;
    offsetMinutes: number;
    iso: string;
  };
}

export function buildNow(instant: Date, timezone: string): NowContext {
  let parts: Intl.DateTimeFormatPart[];
  let zone = timezone;
  try {
    parts = new Intl.DateTimeFormat("en-GB", {
      timeZone: timezone,
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      weekday: "short",
      hour12: false,
    }).formatToParts(instant);
  } catch {
    zone = "UTC";
    parts = new Intl.DateTimeFormat("en-GB", {
      timeZone: "UTC",
      year: "numeric",
      month: "2-digit",
      day: "2-digit",
      hour: "2-digit",
      minute: "2-digit",
      weekday: "short",
      hour12: false,
    }).formatToParts(instant);
  }
  const at = (type: string): string => parts.find((part) => part.type === type)?.value ?? "";
  const [year, month, day] = [Number(at("year")), Number(at("month")), Number(at("day"))];
  const hour = Number(at("hour")) % 24;
  const minute = Number(at("minute"));
  const asUtc = Date.UTC(year, month - 1, day, hour, minute);
  const offsetMinutes = Math.round(
    (asUtc - instant.getTime() + instant.getSeconds() * 1000 + instant.getMilliseconds()) / 60_000,
  );
  const pad = (value: number): string => String(value).padStart(2, "0");
  return {
    utc: instant.toISOString(),
    timezone: zone,
    local: {
      year,
      month,
      day,
      hour,
      minute,
      weekday: at("weekday"),
      offsetMinutes,
      iso: `${year}-${pad(month)}-${pad(day)}T${pad(hour)}:${pad(minute)}`,
    },
  };
}

export const FORMAT_SOURCE = `
const format = {
  number(value, decimals) {
    const fixed = typeof decimals === "number" ? Number(value).toFixed(decimals) : String(Math.round(Number(value)));
    const [whole, fraction] = fixed.split(".");
    const grouped = whole.replace(/\\B(?=(\\d{3})+(?!\\d))/g, ",");
    return fraction === undefined ? grouped : grouped + "." + fraction;
  },
  compact(value) {
    const n = Number(value);
    const units = [[1e9, "B"], [1e6, "M"], [1e3, "K"]];
    for (let i = 0; i < units.length; i++) {
      const [size, suffix] = units[i];
      if (Math.abs(n) >= size) {
        const scaled = n / size;
        const rounded = Math.abs(scaled) >= 100 ? Math.round(scaled) : Math.round(scaled * 10) / 10;
        if (Math.abs(rounded) >= 1000 && i > 0) {
          const [nextSize, nextSuffix] = units[i - 1];
          const nextScaled = n / nextSize;
          const nextRounded = Math.abs(nextScaled) >= 100 ? Math.round(nextScaled) : Math.round(nextScaled * 10) / 10;
          return nextRounded + nextSuffix;
        }
        return rounded + suffix;
      }
    }
    return String(Math.round(n));
  },
  date(local, pattern) {
    const MONTHS = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
    const pad = (v) => String(v).padStart(2, "0");
    return pattern
      .replace("MMM", MONTHS[local.month - 1])
      .replace("yyyy", String(local.year))
      .replace("HH", pad(local.hour))
      .replace("mm", pad(local.minute))
      .replace("dd", pad(local.day))
      .replace("d", String(local.day));
  },
  since(nowIso, thenIso) {
    const seconds = Math.max(0, Math.round((Date.parse(nowIso) - Date.parse(thenIso)) / 1000));
    if (seconds < 60) return "just now";
    if (seconds < 3600) return Math.floor(seconds / 60) + "m ago";
    if (seconds < 86400) return Math.floor(seconds / 3600) + "h ago";
    return Math.floor(seconds / 86400) + "d ago";
  },
};
`;
