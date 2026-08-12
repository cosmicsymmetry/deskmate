import { GlobalRegistrator } from "@happy-dom/global-registrator";

// Registers `window`/`document` globally for `bun test`. Needed only by
// `components.test.tsx`'s `DevicePreview` behaviour tests, which mount a real
// component tree (via `react-dom/client`) so its `useEffect` actually runs — the
// rest of the suite renders with `react-dom/server`'s `renderToStaticMarkup`, which
// never needed a DOM and is unaffected by one being present.
GlobalRegistrator.register();

// Tells React it is safe to batch/flush effects synchronously inside `act()` in this
// environment — without it, React logs "not configured to support act(...)" on every
// `act()` call even though the calls themselves behave correctly.
declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean;
}
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
