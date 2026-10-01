// The clock is supplied by the host. No network or persistent state.
export function plan() {
  return [];
}

function calendar(local) {
  if (!local || !Number.isInteger(local.year) || local.year < 1 || local.year > 9999) {
    throw new Error("Days Left This Year needs a valid date from the host clock.");
  }
  const { year, month, day } = local;
  const leap = year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
  const months = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  if (
    !Number.isInteger(month) ||
    month < 1 ||
    month > 12 ||
    !Number.isInteger(day) ||
    day < 1 ||
    day > months[month - 1]
  ) {
    throw new Error("Days Left This Year needs a valid date from the host clock.");
  }
  // Count civil dates, not 24-hour intervals: DST days need not be 24 hours long.
  let completed = day - 1;
  for (let i = 0; i < month - 1; i++) completed += months[i];
  const total = leap ? 366 : 365;
  return { year, month, day, completed, total, remaining: total - completed };
}

export function render(context) {
  const { year, month, day, completed, total, remaining } = calendar(context?.now?.local);
  const fraction = completed / total;
  const percent = (fraction * 100).toFixed(1);
  const monthName = [
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "May",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
  ][month - 1];
  const face = context?.settings?.face;
  if (face === "squares" || face === "dots") {
    const marks = [];
    for (let index = 0; index < total; index++) {
      const x = 28 + (index % 25) * 16;
      const y = 124 + Math.floor(index / 25) * 13;
      const fill = index < completed ? "#f5f5f7" : "#37373d";
      marks.push(
        face === "squares"
          ? `<rect x="${x}" y="${y}" width="8" height="8" fill="${fill}"/>`
          : `<circle cx="${x + 4}" cy="${y + 4}" r="4" fill="${fill}"/>`,
      );
    }
    return {
      svg: `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368" viewBox="0 0 448 368">
  <rect width="448" height="368" fill="#000000"/>
  <g font-family="Inter" fill="#f5f5f7">
    <text x="24" y="100" font-size="88" font-weight="600">${remaining}</text>
    <text x="212" y="50" font-size="26" font-weight="400">${remaining === 1 ? "day" : "days"} left</text>
    <text x="212" y="80" font-size="26" font-weight="400">in ${year}</text>
    <text x="212" y="104" font-size="16" font-weight="400" fill="#a0a0a8">including today</text>
  </g>
  ${marks.join("\n  ")}
  <g font-family="Inter" font-size="17" font-weight="400" fill="#a0a0a8">
    <text x="24" y="342">${percent}% complete</text>
    <text x="424" y="342" text-anchor="end">${day} ${monthName}</text>
  </g>
</svg>`,
    };
  }
  // Missing settings on existing cards (and unknown values) keep the original face.
  // Pinned panel palette and bundled Inter 400/600, from src/kit/theme.ts.
  // All interpolated content is validated numeric data or a fixed month name.
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368" viewBox="0 0 448 368">
  <rect width="448" height="368" fill="#000000"/>
  <g font-family="Inter" text-anchor="middle">
    <text x="224" y="176" font-size="156" font-weight="600" fill="#f5f5f7">${remaining}</text>
    <text x="224" y="220" font-size="26" font-weight="400" fill="#f5f5f7">${remaining === 1 ? "day" : "days"} left in ${year}</text>
    <text x="224" y="250" font-size="17" font-weight="400" fill="#a0a0a8">including today</text>
  </g>
  <rect x="24" y="288" width="400" height="8" rx="4" fill="#2a2a2f"/>
  ${completed > 0 ? `<rect x="24" y="288" width="${(400 * fraction).toFixed(3)}" height="8" rx="4" fill="#f5f5f7"/>` : ""}
  <g font-family="Inter" font-size="17" font-weight="400" fill="#a0a0a8">
    <text x="24" y="332">${percent}% complete</text>
    <text x="424" y="332" text-anchor="end">${day} ${monthName}</text>
  </g>
</svg>`,
  };
}
