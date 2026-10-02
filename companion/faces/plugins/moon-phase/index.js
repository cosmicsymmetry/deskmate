// Original implementation of Paul Schlyter's solar/lunar orbital formulae:
// https://stjarnhimlen.se/comp/ppcomp.html (sections 3–9).
// Includes the lunar perturbations, rather than assuming a uniform 29.53-day orbit.
// All dates and calculations use the injected UTC clock; no network or state.
const DAY = 86400000;
const EPOCH = Date.UTC(1999, 11, 31);
const RAD = Math.PI / 180;
const sin = (degrees) => Math.sin(degrees * RAD);
const cos = (degrees) => Math.cos(degrees * RAD);
const atan2 = (y, x) => Math.atan2(y, x) / RAD;
const wrap = (degrees) => ((degrees % 360) + 360) % 360;

export function plan() {
  return [];
}

function orbit(mean, eccentricity) {
  const m = wrap(mean) * RAD;
  let e = m;
  // Both orbits have low eccentricity; six bounded Newton steps are ample.
  for (let step = 0; step < 6; step++) {
    e -= (e - eccentricity * Math.sin(e) - m) / (1 - eccentricity * Math.cos(e));
  }
  const x = Math.cos(e) - eccentricity;
  const y = Math.sqrt(1 - eccentricity * eccentricity) * Math.sin(e);
  return { anomaly: atan2(y, x), radius: Math.hypot(x, y) };
}

function position(instant) {
  const d = (instant - EPOCH) / DAY;
  const sunPerigee = 282.9404 + 0.0000470935 * d;
  const sunMean = wrap(356.047 + 0.9856002585 * d);
  const sun = orbit(sunMean, 0.016709 - 0.000000001151 * d);
  const sunLongitude = sun.anomaly + sunPerigee;
  const node = wrap(125.1228 - 0.0529538083 * d);
  const perigee = wrap(318.0634 + 0.1643573223 * d);
  const mean = wrap(115.3654 + 13.0649929509 * d);
  const moon = orbit(mean, 0.0549);
  const argument = moon.anomaly + perigee;
  const x = cos(node) * cos(argument) - sin(node) * sin(argument) * cos(5.1454);
  const y = sin(node) * cos(argument) + cos(node) * sin(argument) * cos(5.1454);
  const z = sin(argument) * sin(5.1454);
  const elongation = node + perigee + mean - (sunMean + sunPerigee);
  const latitudeArgument = perigee + mean;
  const longitude =
    atan2(y, x) -
    1.274 * sin(mean - 2 * elongation) +
    0.658 * sin(2 * elongation) -
    0.186 * sin(sunMean) -
    0.059 * sin(2 * mean - 2 * elongation) -
    0.057 * sin(mean - 2 * elongation + sunMean) +
    0.053 * sin(mean + 2 * elongation) +
    0.046 * sin(2 * elongation - sunMean) +
    0.041 * sin(mean - sunMean) -
    0.035 * sin(elongation) -
    0.031 * sin(mean + sunMean) -
    0.015 * sin(2 * latitudeArgument - 2 * elongation) +
    0.011 * sin(mean - 4 * elongation);
  const latitude =
    atan2(z, Math.hypot(x, y)) -
    0.173 * sin(latitudeArgument - 2 * elongation) -
    0.055 * sin(mean - latitudeArgument - 2 * elongation) -
    0.046 * sin(mean + latitudeArgument - 2 * elongation) +
    0.033 * sin(latitudeArgument + 2 * elongation) +
    0.017 * sin(2 * mean + latitudeArgument);
  const distance =
    moon.radius * 60.2666 - 0.58 * cos(mean - 2 * elongation) - 0.46 * cos(2 * elongation);
  const phase = wrap(longitude - sunLongitude);
  // Earth–Moon–Sun triangle: latitude and finite solar distance both matter.
  const separation = Math.acos(Math.max(-1, Math.min(1, cos(phase) * cos(latitude))));
  const sunDistance = sun.radius * 149597870.7;
  const phaseAngle = Math.atan2(
    sunDistance * Math.sin(separation),
    distance * 6378.14 - sunDistance * Math.cos(separation),
  );
  return { phase, fraction: (1 + Math.cos(phaseAngle)) / 2 };
}

function nextFullMoon(instant) {
  // Find the next 180-degree crossing. A 360->0 new-moon wrap is not a full moon.
  let low = instant;
  let previous = position(low).phase;
  for (let day = 1; day <= 32; day++) {
    let high = instant + day * DAY;
    const current = position(high).phase;
    if (previous < 180 && current >= 180) {
      // Refine to under a second. Model accuracy is minutes, not seconds.
      for (let step = 0; step < 20; step++) {
        const middle = (low + high) / 2;
        if (position(middle).phase < 180) low = middle;
        else high = middle;
      }
      return (low + high) / 2;
    }
    low = high;
    previous = current;
  }
  throw new Error("Moon Phase could not calculate the next full moon.");
}

function moonAt(instant) {
  if (
    !Number.isFinite(instant) ||
    instant < Date.UTC(1900, 0, 1) ||
    instant >= Date.UTC(2100, 0, 1)
  ) {
    throw new Error("Moon Phase needs a valid host clock between 1900 and 2099.");
  }
  const { phase, fraction } = position(instant);
  // Primary phase names cover ±6 degrees (roughly half a day either side).
  // Between those windows the waxing/waning name follows the actual longitude.
  let name;
  if (phase < 6 || phase >= 354) name = "New moon";
  else if (phase < 84) name = "Waxing crescent";
  else if (phase < 96) name = "First quarter";
  else if (phase < 174) name = "Waxing gibbous";
  else if (phase < 186) name = "Full moon";
  else if (phase < 264) name = "Waning gibbous";
  else if (phase < 276) name = "Last quarter";
  else name = "Waning crescent";
  return { phase, fraction, name, nextFull: nextFullMoon(instant) };
}

function moonDisc(fraction, flip) {
  const radius = 102;
  const terminator = 1 - 2 * fraction;
  // Project the lit half of a sphere: a circular limb and elliptical terminator.
  const edge = Math.abs(terminator * radius).toFixed(4);
  const returnArc = terminator >= 0 ? 0 : 1;
  return `<g transform="translate(224 128) scale(${flip} 1)">
    <circle r="102" fill="#1a1a1f" stroke="#2a2a2f" stroke-width="1"/>
    <path d="M 0 -102 A 102 102 0 0 1 0 102 A ${edge} 102 0 0 ${returnArc} 0 -102 Z" fill="#f5f5f7"/>
  </g>`;
}

export function render(context) {
  const instant = typeof context?.now?.utc === "string" ? Date.parse(context.now.utc) : NaN;
  const moon = moonAt(instant);
  const southern = context?.settings?.hemisphere === "south";
  const flip = (moon.phase < 180 ? 1 : -1) * (southern ? -1 : 1);
  const next = new Date(moon.nextFull);
  const month = [
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
  ][next.getUTCMonth()];
  const year =
    next.getUTCFullYear() === new Date(instant).getUTCFullYear() ? "" : ` ${next.getUTCFullYear()}`;
  const date = `${next.getUTCDate()} ${month}${year} UTC`;
  return {
    svg: `<svg xmlns="http://www.w3.org/2000/svg" width="448" height="368" viewBox="0 0 448 368">
  <rect width="448" height="368" fill="#000000"/>
  ${moonDisc(moon.fraction, flip)}
  <g font-family="Inter" text-anchor="middle">
    <text x="224" y="270" font-size="31" font-weight="600" fill="#f5f5f7">${moon.name}</text>
    <text x="224" y="300" font-size="21" font-weight="400" fill="#a0a0a8">${Math.round(moon.fraction * 100)}% illuminated</text>
    <text x="224" y="344" font-size="17" font-weight="400" fill="#a0a0a8">Next full moon · ${date}</text>
  </g>
</svg>`,
  };
}
