export function sourceMapProbe() {
  const origin = location.origin;
  const marker = 'source-map-marker';
  return { origin, marker };
}
