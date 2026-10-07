// Self-contained for QuickJS. Commands are displayed, never executed.
// Each invocation and its platform coverage are audited in README.md.
export const COMMANDS = [
  ["pwd", "Show the current folder's path.", "both"],
  ["ls -lah", "List all files with details and readable sizes.", "both"],
  ["du -sh .", "Show this folder's total disk usage.", "both"],
  ["df -h .", "Show free space on this folder's filesystem.", "both"],
  ["find . -type f -name '*.log'", "Find log files here and in subfolders.", "both", " -name"],
  ["grep -n 'TODO' file.txt", "Find TODO lines and show their line numbers.", "both", " file.txt"],
  ["head -n 20 file.txt", "Read the first 20 lines of a file.", "both"],
  ["tail -n 20 file.txt", "Read the last 20 lines of a file.", "both"],
  ["tail -f app.log", "Watch new log lines; press Ctrl+C to stop.", "both"],
  ["wc -l file.txt", "Count newline characters in a file.", "both"],
  ["sort -u file.txt", "Print sorted lines, with duplicates removed.", "both"],
  ["less file.txt", "Browse a file; press q to quit.", "both"],
  ["file file.txt", "Identify a file's type from its contents.", "both"],
  ["command -v git", "Show how your shell would resolve git.", "both"],
  ["uname -srm", "Show the kernel name, release and architecture.", "both"],
  ["id", "Show your user and group IDs.", "both"],
  ["uptime", "Show uptime and recent system load.", "both"],
  ["ps -ef", "List all processes with their command lines.", "both"],
  ["date '+%Y-%m-%d'", "Print today's date as year-month-day.", "both"],
  ["sed -n '1,20p' file.txt", "Print lines 1 to 20 without editing the file.", "both", " file.txt"],
  ["tar -tf archive.tar", "List an archive's contents without extracting.", "both"],
  ["diff -u old.txt new.txt", "Compare two files with surrounding lines.", "both", " new.txt"],
  ["man ls", "Read the ls manual; press q to quit the pager.", "both"],
  ["printenv PATH", "Show the folders searched for commands.", "both"],
  ["free -h", "Show memory and swap usage in readable units.", "linux"],
  ["lsblk -f", "List block devices and their filesystems.", "linux"],
  ["date -d tomorrow '+%F'", "Print tomorrow's date as year-month-day.", "linux", " '+%F'"],
  ["sw_vers", "Show the macOS version and build number.", "macos"],
  ["pmset -g batt", "Show the power source and battery status.", "macos"],
  ["date -v+1d '+%F'", "Print tomorrow's date as year-month-day.", "macos"],
].map(([command, explanation, platform, split]) => ({ command, explanation, platform, split }));

function selection(context) {
  const local = context?.now?.local;
  const year = local?.year;
  const month = local?.month;
  const day = local?.day;
  const date = `${String(year).padStart(4, "0")}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
  const instant = new Date(`${date}T12:00:00Z`);
  if (
    !Number.isInteger(year) ||
    year < 1 ||
    year > 9999 ||
    !Number.isInteger(month) ||
    !Number.isInteger(day) ||
    !Number.isFinite(instant.getTime()) ||
    instant.toISOString().slice(0, 10) !== date
  ) {
    throw new Error("Terminal Command needs a valid local date from the host clock.");
  }
  const platform = ["linux", "macos"].includes(context?.settings?.platform)
    ? context.settings.platform
    : "both";
  // Both mixes systems. The footer always names the selected entry's actual support.
  const deck = COMMANDS.filter(
    (item) => platform === "both" || item.platform === "both" || item.platform === platform,
  );
  const serial = Math.floor(instant.getTime() / 86400000);
  const cycle = Math.floor(serial / deck.length);
  let seed = 2166136261;
  for (const character of `${platform}:${cycle}`)
    seed = Math.imul(seed ^ character.charCodeAt(0), 16777619) >>> 0;
  for (let i = deck.length - 1; i > 0; i--) {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    const j = seed % (i + 1);
    [deck[i], deck[j]] = [deck[j], deck[i]];
  }
  const old = context.state;
  const zone = context.now.timezone;
  const offset =
    old?.date === date &&
    old?.zone === zone &&
    old?.platform === platform &&
    Number.isInteger(old.offset) &&
    old.offset >= 0 &&
    old.offset < deck.length
      ? old.offset
      : 0;
  const taps =
    Number.isSafeInteger(context.event?.taps) && context.event.taps > 0
      ? context.event.taps % deck.length
      : 0;
  const next = (offset + taps) % deck.length;
  const index = ((serial % deck.length) + deck.length + next) % deck.length;
  return { item: deck[index], state: { date, zone, platform, offset: next } };
}

function commandLines(item) {
  if (!item.split) return [item.command];
  const at = item.command.indexOf(item.split);
  return [`${item.command.slice(0, at)} \\`, item.command.slice(at + 1)];
}

function requests(item) {
  const words = item.explanation.split(" ");
  const breaks = [];
  for (let at = 1; at < words.length; at++) {
    breaks.push([words.slice(0, at).join(" "), words.slice(at).join(" ")]);
  }
  return {
    lines: commandLines(item),
    breaks,
    measure: [
      ...commandLines(item).map((text) => ({ text, size: 44, weight: 600 })),
      ...[item.explanation, ...breaks.flat()].map((text) => ({ text, size: 26, weight: 400 })),
    ],
  };
}

export function plan(context) {
  if (!context?.now || context.answers?.length) return [];
  return [{ measure: requests(selection(context).item).measure }];
}

function xml(value) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

export function render(context) {
  const { item, state } = selection(context);
  const { lines, breaks, measure } = requests(item);
  const widths = context.answers?.[0]?.measurements?.map((value) => value.width);
  if (
    !widths ||
    widths.length !== measure.length ||
    widths.some((width) => !Number.isFinite(width) || width < 0)
  ) {
    throw new Error("Terminal Command could not measure its text. Try again shortly.");
  }
  const commandWidth = Math.max(...widths.slice(0, lines.length));
  const size = [44, 40, 36, 32].find((value) => (commandWidth * value) / 44 <= 384);
  let explanation = [item.explanation];
  if (widths[lines.length] > 384) {
    let best = Infinity;
    explanation = [];
    for (let i = 0; i < breaks.length; i++) {
      const left = widths[lines.length + 1 + i * 2];
      const right = widths[lines.length + 2 + i * 2];
      if (Math.max(left, right) <= 384 && Math.abs(left - right) < best) {
        best = Math.abs(left - right);
        explanation = breaks[i];
      }
    }
  }
  if (!size || !explanation.length)
    throw new Error("Terminal Command text does not fit the panel.");
  // Integer document coordinates need no fractional formatting (fixed/toFixed).
  // No letter-spacing: measured and drawn advances use the same bundled Inter.
  const command = lines
    .map(
      (line, index) =>
        `<text x="32" y="${lines.length === 1 ? 148 : 116 + index * 52}" font-size="${size}" font-weight="600">${xml(line)}</text>`,
    )
    .join("");
  const description = explanation
    .map((line, index) => `<text x="32" y="${232 + index * 34}" font-size="26">${xml(line)}</text>`)
    .join("");
  const platform = { both: "Linux + macOS", linux: "Linux", macos: "macOS" }[item.platform];
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368" viewBox="0 0 448 368"><rect width="448" height="368" fill="#000000"/><g font-family="Inter" fill="#f5f5f7">${command}${description}<g font-size="18" fill="#a0a0a8"><text x="32" y="336">${platform}</text><text x="416" y="336" text-anchor="end">Tap for next</text></g></g></svg>`,
    state,
  };
}
