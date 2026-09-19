// Query imports give stateful mock clients an independent module instance in Bun.
declare module "*?characterization" {
  const backend: typeof import("../src/dev/backendClient");
  export = backend;
}

declare module "*?production-client" {
  const backend: typeof import("../src/lib/backendClient");
  export = backend;
}
