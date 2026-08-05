import type { AppConfig, ScreenSettings, ValidationIssue, WidgetSettings } from "./types";

export type WidgetKind = "clock" | "pomodoro" | "calendar";

export function screenWidgetIds(screen: ScreenSettings): string[] {
  return screen.layout.kind === "single"
    ? [screen.layout.widget_id]
    : screen.layout.tiles.map((tile) => tile.widget_id);
}

export function primaryScreenWidgetId(screen: ScreenSettings): string | null {
  return screenWidgetIds(screen)[0] ?? null;
}

export function copyConfig(config: AppConfig): AppConfig {
  return {
    ...config,
    preferences: { ...config.preferences },
    widgets: config.widgets.map((widget) => ({
      ...widget,
      ...(widget.kind === "calendar" ? { source: { ...widget.source } } : {}),
      ...(widget.kind === "json-feed"
        ? { mappings: widget.mappings.map((mapping) => ({ ...mapping })) }
        : {}),
      template: { ...widget.template },
      tap_action: { ...widget.tap_action },
      refresh: { ...widget.refresh },
    })) as WidgetSettings[],
    screens: config.screens.map((screen) => ({
      ...screen,
      layout:
        screen.layout.kind === "single"
          ? { ...screen.layout }
          : { ...screen.layout, tiles: screen.layout.tiles.map((tile) => ({ ...tile })) },
    })),
    assets: config.assets.map((asset) => ({
      ...asset,
      source: { ...asset.source },
      kind:
        asset.kind.kind === "icon"
          ? { ...asset.kind }
          : {
              ...asset.kind,
              glyph_ranges: asset.kind.glyph_ranges.map((range) => ({ ...range })),
            },
    })),
    carousel: { ...config.carousel },
    updater: { ...config.updater },
  };
}

export function widgetName(widget: WidgetSettings): string {
  switch (widget.kind) {
    case "clock":
      return widget.title || "Digital clock";
    case "pomodoro":
      return widget.label || "Pomodoro";
    case "calendar":
      return widget.title || "Calendar";
    case "weather":
    case "json-feed":
    case "rss":
      return widget.title || widget.kind;
  }
}

export function widgetKindName(kind: WidgetSettings["kind"]): string {
  switch (kind) {
    case "clock":
      return "Digital clock";
    case "pomodoro":
      return "Pomodoro";
    case "calendar":
      return "ICS calendar";
    case "weather":
      return "Weather";
    case "json-feed":
      return "JSON feed";
    case "rss":
      return "RSS feed";
  }
}

function nextId(prefix: string, used: Set<string>): string {
  if (!used.has(prefix)) {
    return prefix;
  }
  let suffix = 2;
  while (used.has(`${prefix}-${suffix}`)) {
    suffix += 1;
  }
  return `${prefix}-${suffix}`;
}

export function addWidget(
  config: AppConfig,
  kind: WidgetKind,
): {
  config: AppConfig;
  widgetId: string;
} {
  const next = copyConfig(config);
  const widgetIds = new Set(next.widgets.map((widget) => widget.id));
  const widgetId = nextId(kind, widgetIds);
  const screenIds = new Set(next.screens.map((screen) => screen.id));
  const screenId = nextId(`${widgetId}-screen`, screenIds);

  let widget: WidgetSettings;
  switch (kind) {
    case "clock":
      widget = {
        kind,
        id: widgetId,
        size: "full",
        title: "Desk",
        show_seconds: true,
        template: { kind: "digital-clock" },
        tap_action: { kind: "none" },
        refresh: { kind: "device-local" },
        interrupt_policy: "disabled",
      };
      break;
    case "pomodoro":
      widget = {
        kind,
        id: widgetId,
        size: "standard",
        label: "Focus",
        duration_seconds: 25 * 60,
        template: { kind: "progress-ring" },
        tap_action: { kind: "start-pause" },
        refresh: { kind: "device-local" },
        interrupt_policy: "enabled",
      };
      break;
    case "calendar":
      widget = {
        kind,
        id: widgetId,
        size: "standard",
        title: "Up next",
        source: { kind: "url", value: "" },
        template: { kind: "row-list" },
        tap_action: { kind: "none" },
        refresh: { kind: "interval", minutes: 15 },
        interrupt_policy: "disabled",
      };
      break;
  }

  next.widgets.push(widget);
  next.screens.push({ id: screenId, layout: { kind: "single", widget_id: widgetId } });
  return { config: next, widgetId };
}

export function updateWidget(
  config: AppConfig,
  widgetId: string,
  replacement: WidgetSettings,
): AppConfig {
  return {
    ...config,
    widgets: config.widgets.map((widget) => (widget.id === widgetId ? replacement : widget)),
  };
}

export function removeWidget(config: AppConfig, widgetId: string): AppConfig {
  return {
    ...config,
    widgets: config.widgets.filter((widget) => widget.id !== widgetId),
    screens: config.screens.filter((screen) => !screenWidgetIds(screen).includes(widgetId)),
  };
}

export function moveScreen(
  screens: ScreenSettings[],
  screenId: string,
  targetIndex: number,
): ScreenSettings[] {
  const sourceIndex = screens.findIndex((screen) => screen.id === screenId);
  if (sourceIndex < 0 || screens.length < 2) {
    return screens;
  }
  const boundedTarget = Math.max(0, Math.min(targetIndex, screens.length - 1));
  if (sourceIndex === boundedTarget) {
    return screens;
  }
  const reordered = [...screens];
  const [moved] = reordered.splice(sourceIndex, 1);
  reordered.splice(boundedTarget, 0, moved);
  return reordered;
}

export function screenMoveFromKey(key: string, altKey: boolean): -1 | 0 | 1 {
  if (!altKey) {
    return 0;
  }
  if (key === "ArrowUp") {
    return -1;
  }
  if (key === "ArrowDown") {
    return 1;
  }
  return 0;
}

export function issuesForPath(issues: ValidationIssue[], path: string): ValidationIssue[] {
  return issues.filter((issue) => issue.path === path || issue.path.startsWith(`${path}.`));
}

export function firstSelectableWidget(config: AppConfig): string | null {
  const firstScreen = config.screens[0];
  const firstWidgetId = firstScreen ? primaryScreenWidgetId(firstScreen) : null;
  if (firstWidgetId && config.widgets.some((widget) => widget.id === firstWidgetId)) {
    return firstWidgetId;
  }
  return config.widgets[0]?.id ?? null;
}

export function needsFirstRunGuidance(config: AppConfig): boolean {
  const kinds = new Set(config.widgets.map((widget) => widget.kind));
  return !kinds.has("pomodoro") || !kinds.has("calendar");
}
