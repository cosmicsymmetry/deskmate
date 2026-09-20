/**
 * An age the way a panel should state it: coarse, short, and never more precise
 * than it is honest about. A panel refreshed every fifteen minutes cannot claim
 * "3 minutes ago" and mean it, so the units step up quickly. An item dated in the
 * future -- which feeds do produce, usually via a timezone mistake -- reads as
 * "now" rather than as a negative age.
 */
export function relativeAge(published: Date, now: Date): string {
  const seconds = Math.trunc((now.getTime() - published.getTime()) / 1000);
  if (seconds <= 60) {
    return "now";
  }
  const minutes = Math.trunc(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m`;
  }
  const hours = Math.trunc(minutes / 60);
  if (hours < 24) {
    return `${hours}h`;
  }
  const days = Math.trunc(hours / 24);
  if (days < 7) {
    return `${days}d`;
  }
  if (days < 365) {
    return `${Math.trunc(days / 7)}w`;
  }
  return `${Math.trunc(days / 365)}y`;
}
